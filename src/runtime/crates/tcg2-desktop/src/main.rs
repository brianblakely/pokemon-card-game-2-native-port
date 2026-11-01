//! `pokemon-tcg2`, the app on Linux, macOS and Windows (the library has it
//! all; Android loads the library itself).

// A window, not a console, on Windows (errors go to a message box).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    tcg2_desktop::start(std::env::args().skip(1)).into()
}
