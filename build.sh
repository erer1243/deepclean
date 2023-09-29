#!/bin/sh
cd "$(dirname "$0")"
cargo build --release
cp target/release/deepclean ./deepclean
strip ./deepclean
