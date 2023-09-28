use regex::Regex;
use std::{
    convert::identity,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

fn main() -> io::Result<()> {
    let rust_proj = Pattern::new("Rust project")
        .files_exist(["Cargo.toml"])
        .dirs_exist(["target"])
        .clean_commands(["cargo clean"]);

    let makefile_clean_proj = Pattern::new("Makefile with clean target")
        .files_exist(["Makefile|makefile"])
        .check_commands(["make clean --dry-run"])
        .clean_commands(["make clean"]);

    let pats = [rust_proj, makefile_clean_proj];

    for arg in std::env::args().skip(1) {
        let mut path = PathBuf::from(&arg);
        if !path.is_absolute() {
            path = path.canonicalize()?;
        }

        for p in &pats {
            match p.match_dir(&path) {
                Ok(true) => println!("{}: {arg}", p.name),
                Ok(false) => (),
                Err(e) => eprintln!("{arg}: {e}"),
            }
        }
    }

    Ok(())
}

#[derive(Default, Clone)]
struct Pattern {
    name: Box<str>,
    files_exist: Box<[Regex]>,
    dirs_exist: Box<[Regex]>,
    check_commands: Box<[String]>,
    clean_commands: Box<[String]>,
}

impl Pattern {
    fn new(name: impl Into<Box<str>>) -> Self {
        Pattern {
            name: name.into(),
            ..Pattern::default()
        }
    }

    fn match_dir(&self, d: impl AsRef<Path>) -> io::Result<bool> {
        debug_assert!(d.as_ref().is_absolute(), "match_dir on absolute path");

        let rd = fs::read_dir(&d)?;

        let mut n_matched_files = 0;
        let mut n_matched_dirs = 0;
        for f in rd {
            let f = f?;
            let ty = f.file_type()?;
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

        for c in self.check_commands.iter() {
            let status = Command::new("sh")
                .arg("-x")
                .arg("-c")
                .arg(c)
                .current_dir(&d)
                .status()?;

            if !status.success() {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

macro_rules! setter {
    ($name:ident, $cls:expr) => {
        fn $name(mut self, ii: impl IntoIterator<Item = impl Into<String>>) -> Self {
            self.$name = ii.into_iter().map(Into::into).map($cls).collect();
            self
        }
    };
}

impl Pattern {
    setter!(files_exist, str_to_regex);
    setter!(dirs_exist, str_to_regex);
    setter!(check_commands, identity);
    setter!(clean_commands, identity);
}

fn str_to_regex(s: String) -> Regex {
    let s2 = format!("^({s})$");
    Regex::new(&s2).unwrap_or_else(|e| panic!("Compiling regex: `{s}`\nError: {e}"))
}

// #[derive(Clone)]
// struct DirEntry {
//     name: String,
//     file_type: fs::FileType,
// }

// impl DirEntry {
//     fn new(src: fs::DirEntry) -> io::Result<Self> {
//         let name = src
//             .file_name()
//             .into_string()
//             .unwrap_or_else(|s| s.to_string_lossy().into_owned());
//         Ok(DirEntry {
//             name,
//             file_type: src.file_type()?,
//         })
//     }
// }
