//! Generates the export list for Windows from `exports.txt`. On 32-bit x86
//! games import decorated stdcall names (`_grDrawTriangle@12`); on 64-bit
//! there is no decoration.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=exports.txt");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let x86 = env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let table = fs::read_to_string("exports.txt").expect("exports.txt");
    let mut def = String::from("LIBRARY glide2x\nEXPORTS\n");
    for line in table.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let mut parts = line.split_whitespace();
        let (name, bytes) = (parts.next().expect("name"), parts.next().expect("bytes"));
        if x86 {
            // Export the decorated name games import. MSVC's link.exe keeps it only
            // when the entry names the decorated symbol itself; with an undecorated
            // internal name it drops the `@N`. GNU ld needs the `= name@N` mapping.
            if msvc {
                def.push_str(&format!("    _{name}@{bytes}\n"));
            } else {
                def.push_str(&format!("    _{name}@{bytes} = {name}@{bytes}\n"));
            }
        } else {
            def.push_str(&format!("    {name}\n"));
        }
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("glide2x.def");
    fs::write(&out, def).expect("write .def");
    if msvc {
        println!("cargo:rustc-cdylib-link-arg=/DEF:{}", out.display());
    } else {
        println!("cargo:rustc-cdylib-link-arg={}", out.display());
    }
}
