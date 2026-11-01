//! Translates src/decomp into the kernel: an annotated build of the decomp
//! (byte-identical, checked against rom.sha1) gives the program and its
//! code/data boundaries, and tcg2-recomp writes it as Rust into OUT_DIR.
//! Needs RGBDS, make and a C compiler (the Nix shell has them).
//!
//! `TCG2_KERNEL_DIR` instead takes sources tcg2-recomp already wrote
//! (`tcg2-recomp --out DIR`, on another machine), so platforms without the
//! decomp's toolchain can build the kernel.

use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo::rerun-if-env-changed=TCG2_KERNEL_DIR");
    println!("cargo::rerun-if-env-changed=TCG2_DECOMP");
    let files = if let Some(dir) = std::env::var_os("TCG2_KERNEL_DIR").map(PathBuf::from) {
        println!("cargo::rerun-if-changed={}", dir.display());
        let mut files = Vec::new();
        for entry in
            fs::read_dir(&dir).unwrap_or_else(|e| panic!("TCG2_KERNEL_DIR {}: {e}", dir.display()))
        {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".rs") || name == "report.txt" {
                files.push((name, fs::read_to_string(&path).unwrap()));
            }
        }
        assert!(
            files.iter().any(|(n, _)| n == "kernel.rs"),
            "TCG2_KERNEL_DIR has no kernel.rs"
        );
        files
    } else {
        let decomp = std::env::var_os("TCG2_DECOMP")
            .map(PathBuf::from)
            .unwrap_or_else(|| tcg2_recomp::decomp_dir(&manifest));
        let decomp = decomp
            .canonicalize()
            .unwrap_or_else(|e| panic!("src/decomp not found at {}: {e}", decomp.display()));
        for input in ["src", "tools", "Makefile", "rom.sha1", ".rgbds-version"] {
            println!("cargo::rerun-if-changed={}", decomp.join(input).display());
        }
        let prepared =
            tcg2_recomp::prepare(&decomp, &out.join("decomp")).unwrap_or_else(|e| panic!("{e}"));
        let translation = tcg2_recomp::translate(&prepared, &tcg2_recomp::RAM_ROUTINES)
            .unwrap_or_else(|e| panic!("{e}"));
        let mut files = translation.files;
        files.push(("report.txt".to_string(), translation.report));
        files
    };
    for (name, text) in &files {
        let path = out.join(name);
        if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            fs::write(&path, text).unwrap();
        }
    }
}
