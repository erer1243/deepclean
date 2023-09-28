use regex::Regex;
use std::{
    fs,
    io::{self, stderr, stdout, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{exit, Command, ExitStatus},
};

fn usage() -> ! {
    // UNWRAP: If this fails, there is no executable name. Something would be terribly off.
    let exec_name = std::env::args().nth(0).unwrap();
    indoc::eprintdoc! {"
        Usage: {exec_name} [OPTIONS] DIR
          -n, --dry-run   Skip running cleanup commands in matched directories.
                          This may search directories that would've been cleaned up otherwise,
                          resulting in different matches than normal.
          -h, --help      Show this message
    "};
    exit(1)
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();

    if args.len() == 1 {
        usage()
    }

    let mut dry_run = false;

    for arg in args.iter().filter(|s| s.starts_with('-')) {
        match arg.as_str() {
            "-n" | "--dry-run" => dry_run = true,
            _ => usage(),
        }
    }

    let rust_proj = Pattern::new("built Rust project")
        .files_exist(["Cargo.toml"])
        .dirs_exist(["target"])
        .clean_commands(["cargo clean"]);

    let makefile_clean_proj = Pattern::new("Makefile with clean target")
        .files_exist(["Makefile|makefile"])
        .check_commands(["make clean --dry-run"])
        .clean_commands(["make clean"]);

    let has_pycache = Pattern::new("contains __pycache__/")
        .dirs_exist(["__pycache__"])
        .clean_commands(["rm -r __pycache__"]);

    let has_compiled_pyth = Pattern::new("contains compiled python")
        .files_exist([r".*\.py[co]"])
        .clean_commands(["rm *.pyc *.pyo"]);

    let pats = [
        rust_proj,
        makefile_clean_proj,
        has_pycache,
        has_compiled_pyth,
    ];

    let mut non_flag_args = args.iter().skip(1).filter(|s| !s.starts_with('-'));
    let (Some(root_dir), None) = (non_flag_args.next(), non_flag_args.next()) else {
        usage();
    };

    let meta = match fs::metadata(root_dir) {
        Ok(meta) => meta,
        Err(e) => {
            eprintln!("Getting metadata for `{root_dir}`: {e}");
            exit(1);
        }
    };

    if !meta.is_dir() {
        eprintln!("`{root_dir}` is not a directory");
        exit(1);
    }

    // UNWRAP: Since fs::metadata succeeded earlier, this should succeed
    let root_dir = fs::canonicalize(root_dir).unwrap();

    macro_rules! print_subdir {
        ($stream_func:expr, $path:expr) => {{
            let short = match $path.strip_prefix(&root_dir) {
                Ok(p) if p.as_os_str().is_empty() => root_dir.as_ref(),
                Ok(p) => p,
                Err(_) => &$path,
            };
            let mut stream = $stream_func();
            // UNWRAP: These unwraps match std print macro behavior
            stream.write(short.as_os_str().as_bytes()).unwrap();
            stream.flush().unwrap();
        }};
    }

    let mut stk: Vec<PathBuf> = vec![root_dir.clone()];
    while let Some(dir) = stk.pop() {
        for pat in &pats {
            match pat.match_dir(&dir) {
                Ok(false) => continue,
                Ok(true) => {
                    print_subdir!(stdout, dir);
                    println!(" matched '{}'", pat.name);
                }
                Err(e) => {
                    eprint!("Matching '{}' on `", pat.name);
                    print_subdir!(stderr, dir);
                    eprintln!("`: {e}");
                    break;
                }
            }

            if dry_run {
                continue;
            }

            let err_msg = match pat.clean_dir(&dir) {
                Ok(true) => continue,
                Ok(false) => "Exit status was non-zero".to_string(),
                Err(e) => e.to_string(),
            };

            eprint!("Clean commands for '{}' in `", pat.name);
            print_subdir!(stderr, dir);
            eprintln!("`: {err_msg}");
        }

        // UNWRAP: These unwraps will likely work because match_dir would have just checked this,
        // but this should be handled (TODO)
        for f in fs::read_dir(&dir).unwrap() {
            let f = f.unwrap();
            let ty = f.file_type().unwrap();

            if ty.is_dir() {
                stk.push(f.path());
            }
        }
    }
}

#[derive(Default, Clone)]
struct Pattern {
    name: Box<str>,
    files_exist: Box<[Regex]>,
    dirs_exist: Box<[Regex]>,
    check_commands: Box<[Box<str>]>,
    clean_commands: Box<[Box<str>]>,
}

impl Pattern {
    fn new(name: impl Into<Box<str>>) -> Self {
        Pattern {
            name: name.into(),
            ..Pattern::default()
        }
    }

    fn match_dir(&self, d: &Path) -> io::Result<bool> {
        debug_assert!(d.is_absolute(), "match_dir on absolute path");

        // Match against files_exist and dirs_exist
        let rd = fs::read_dir(d)?;
        let mut n_matched_files = 0;
        let mut n_matched_dirs = 0;
        for f in rd {
            let f = f?;
            let ty = f.file_type()?;

            if !(ty.is_file() || ty.is_dir()) {
                continue;
            }

            let name = f.file_name();
            let name = name.to_string_lossy();
            let name_matches = |re: &Regex| re.is_match(&name);

            if ty.is_file() && self.files_exist.iter().any(name_matches) {
                n_matched_files += 1;
            } else if ty.is_dir() && self.dirs_exist.iter().any(name_matches) {
                n_matched_dirs += 1;
            }
        }

        if n_matched_files < self.files_exist.len() || n_matched_dirs < self.dirs_exist.len() {
            return Ok(false);
        }

        // Run check commands
        for c in self.check_commands.iter() {
            if !run_command_in_dir(c, d)?.success() {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn clean_dir(&self, d: &Path) -> io::Result<bool> {
        debug_assert!(d.is_absolute(), "clean_dir on absolute path");

        // Run clean commands
        for c in self.clean_commands.iter() {
            if !run_command_in_dir(c, d)?.success() {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

fn run_command_in_dir(cmd: &str, dir: &Path) -> io::Result<ExitStatus> {
    Command::new("sh")
        .args(["-x", "-c"])
        .arg(cmd)
        .current_dir(dir)
        .status()
}

macro_rules! pattern_setters {
    ($($name:ident: $fn:expr),* $(,)?) => {
        impl Pattern {
            $(
                fn $name(mut self, ii: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
                    self.$name = ii.into_iter().map($fn).collect();
                    self
                }
            )*
        }
    };
}

pattern_setters! {
    files_exist: str_to_regex,
    dirs_exist: str_to_regex,
    check_commands: str_to_string,
    clean_commands: str_to_string,
}

fn str_to_string(s: impl AsRef<str>) -> Box<str> {
    s.as_ref().to_owned().into_boxed_str()
}

fn str_to_regex(s: impl AsRef<str>) -> Regex {
    let s2 = format!("^({})$", s.as_ref());
    Regex::new(&s2).unwrap_or_else(|e| panic!("Compiling regex: `{}`\nError: {e}", s.as_ref()))
}
