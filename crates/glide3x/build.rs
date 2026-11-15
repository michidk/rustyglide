//! Generates the Windows export list from `exports.txt`.
use std::{env, fs, path::PathBuf};

include!("../glide-abi/def.rs");

fn main() {
    write_def("glide3x");
}
