//! `tcg2-recomp [--decomp DIR] [--work DIR] --out DIR`: builds the annotated
//! decomp and writes the translated kernel sources and a report, as the
//! kernel's build script does, for reading and diffing.

use std::path::PathBuf;

fn main() {
    let mut decomp = PathBuf::from("src/decomp");
    let mut work = PathBuf::from("build/decomp");
    let mut out: Option<PathBuf> = None;
    let mut dump_seeds: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || PathBuf::from(args.next().unwrap_or_else(|| usage(&arg)));
        match arg.as_str() {
            "--decomp" => decomp = value(),
            "--work" => work = value(),
            "--out" => out = Some(value()),
            "--dump-seeds" => dump_seeds = Some(value()),
            _ => usage(&arg),
        }
    }
    let out = out.unwrap_or_else(|| usage("--out"));
    let prepared = tcg2_recomp::prepare(&decomp, &work).unwrap_or_else(|e| fail(&e));
    if let Some(path) = dump_seeds {
        let seeds = tcg2_recomp::seeds(&prepared);
        let mut text = String::new();
        for (bank, addr, why) in &seeds.code {
            text.push_str(&format!("code {bank:02x}:{addr:04x} {why}\n"));
        }
        for (bank, addr) in &seeds.data {
            text.push_str(&format!("data {bank:02x}:{addr:04x}\n"));
        }
        std::fs::write(path, text).unwrap_or_else(|e| fail(&e.to_string()));
    }
    let translation =
        tcg2_recomp::translate(&prepared, &tcg2_recomp::RAM_ROUTINES).unwrap_or_else(|e| fail(&e));
    std::fs::create_dir_all(&out).unwrap_or_else(|e| fail(&e.to_string()));
    for (name, text) in &translation.files {
        std::fs::write(out.join(name), text).unwrap_or_else(|e| fail(&e.to_string()));
    }
    std::fs::write(out.join("report.txt"), &translation.report)
        .unwrap_or_else(|e| fail(&e.to_string()));
    print!("{}", translation.report);
}

fn usage(arg: &str) -> ! {
    eprintln!(
        "tcg2-recomp: bad argument {arg:?}\nusage: tcg2-recomp [--decomp DIR] [--work DIR] --out DIR"
    );
    std::process::exit(2)
}

fn fail(message: &str) -> ! {
    eprintln!("tcg2-recomp: {message}");
    std::process::exit(1)
}
