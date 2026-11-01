//! The player's settings: `settings.ini` beside the app, `key = value` lines.

use std::path::{Path, PathBuf};
use tcg2_kernel::Language;
use tcg2_render::{Filter, Fit, Look};

/// How the game clock is paced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    /// The console's own rate, 59.7275 frames per second.
    Exact,
    /// The display's refresh rate when it is within 1.5% of the console's
    /// (a 60 Hz display shows every frame once, without a periodic repeat).
    Display,
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// The game's language.
    pub language: Language,
    pub look: Look,
    pub fullscreen: bool,
    /// 0-100.
    pub volume: u8,
    pub window: (i32, i32),
    pub speed: Speed,
    /// The most pictures presented per second (0: every new picture). The
    /// game itself always runs at full rate.
    pub max_fps: u32,
    /// Stop the game while the window is minimised or hidden.
    pub pause_hidden: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::default(),
            look: Look::default(),
            fullscreen: false,
            volume: 100,
            window: (160 * 4, 144 * 4),
            speed: Speed::Exact,
            max_fps: 0,
            pause_hidden: true,
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut settings = Settings::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return settings;
        };
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            settings.set(key.trim(), value.trim());
        }
        settings
    }

    /// Applies one `key = value` (from the file or the command line);
    /// false for an unknown key or value.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        match key {
            "language" => Language::parse(value).map(|l| self.language = l).is_some(),
            "filter" => Filter::parse(value).map(|f| self.look.filter = f).is_some(),
            "fit" => Fit::parse(value).map(|f| self.look.fit = f).is_some(),
            "lcd_colours" => parse_bool(value)
                .map(|b| self.look.lcd_colours = b)
                .is_some(),
            "frame_blending" => parse_bool(value)
                .map(|b| self.look.frame_blending = b)
                .is_some(),
            "fullscreen" => parse_bool(value).map(|b| self.fullscreen = b).is_some(),
            "pause_hidden" => parse_bool(value).map(|b| self.pause_hidden = b).is_some(),
            "volume" => value
                .parse::<u8>()
                .ok()
                .map(|v| self.volume = v.min(100))
                .is_some(),
            "max_fps" => value
                .parse::<u32>()
                .ok()
                .map(|v| self.max_fps = v)
                .is_some(),
            "speed" => match value {
                "exact" => {
                    self.speed = Speed::Exact;
                    true
                }
                "display" => {
                    self.speed = Speed::Display;
                    true
                }
                _ => false,
            },
            "window" => value
                .split_once('x')
                .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
                .map(|(w, h): (i32, i32)| self.window = (w.max(160), h.max(144)))
                .is_some(),
            _ => false,
        }
    }

    pub fn save(&self, path: &Path) {
        let text = format!(
            "# Pokémon Card GB2 settings\n\
             # language: en-US (American English) | ja (Japanese, the original)\n\
             language = {}\n\
             # filter: sharp | nearest | smooth\n\
             filter = {}\n\
             # fit: aspect | integer | stretch\n\
             fit = {}\n\
             lcd_colours = {}\n\
             frame_blending = {}\n\
             fullscreen = {}\n\
             window = {}x{}\n\
             volume = {}\n\
             # speed: exact (59.73 Hz, the console's) | display (match a ~60 Hz display)\n\
             speed = {}\n\
             # the most pictures shown per second (0 = every new picture)\n\
             max_fps = {}\n\
             pause_hidden = {}\n",
            self.language.tag(),
            self.look.filter.name(),
            self.look.fit.name(),
            self.look.lcd_colours,
            self.look.frame_blending,
            self.fullscreen,
            self.window.0,
            self.window.1,
            self.volume,
            match self.speed {
                Speed::Exact => "exact",
                Speed::Display => "display",
            },
            self.max_fps,
            self.pause_hidden,
        );
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("pokemon-tcg2: {}: {e}", path.display());
        }
    }
}

/// Where the app keeps its files: beside the executable (beside the `.app`
/// on macOS) as a portable folder holding `rom/`, `saves/`, `screenshots/`
/// and `settings.ini`; or, when that folder cannot be written (an installed
/// package in /usr/bin, say), the platform's per-user data folder
/// (`~/.local/share/pokemon-tcg2`, `~/Library/Application Support/...`,
/// `%APPDATA%\...`). `TCG2_HOME` overrides both.
pub fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("TCG2_HOME") {
        return PathBuf::from(dir);
    }
    let portable = portable_dir();
    if let Some((dir, bundled)) = &portable {
        // Beside an .app (often /Applications) only when the player made
        // the portable folder themselves; beside a plain executable also
        // when the folder can be written.
        if dir.join("rom").is_dir() || (!bundled && writable(dir)) {
            return dir.clone();
        }
    }
    user_data_dir()
        .or(portable.map(|(dir, _)| dir))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The folder beside the executable (beside the `.app` on macOS) and
/// whether it is an app bundle's.
fn portable_dir() -> Option<(PathBuf, bool)> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    // Foo.app/Contents/MacOS/pokemon-tcg2 -> the folder holding Foo.app.
    if dir.ends_with("Contents/MacOS") {
        return Some((dir.parent()?.parent()?.parent()?.to_path_buf(), true));
    }
    Some((dir.to_path_buf(), false))
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".tcg2-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn user_data_dir() -> Option<PathBuf> {
    // SAFETY: SDL_GetPrefPath needs no initialisation; the string it returns
    // is freed with SDL_free.
    unsafe {
        let org = std::ffi::CString::new("pokemon-card-game-2-native-port").ok()?;
        let app = std::ffi::CString::new("pokemon-tcg2").ok()?;
        let path = sdl3_sys::everything::SDL_GetPrefPath(org.as_ptr(), app.as_ptr());
        if path.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr(path)
            .to_string_lossy()
            .into_owned();
        sdl3_sys::everything::SDL_free(path.cast());
        Some(PathBuf::from(dir))
    }
}
