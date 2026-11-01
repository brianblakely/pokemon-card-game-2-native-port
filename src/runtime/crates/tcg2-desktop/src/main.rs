//! Pokémon Card GB2 on the desktop.
//!
//! The kernel (the game's own program, translated to Rust) runs one frame
//! per 1/59.73 s in a burst of native code, then the thread sleeps in SDL's
//! event wait until the next frame is due or the player does something. A
//! picture is drawn only when it changed, and the sound device idles when
//! the game makes no sound: a static screen costs a few microseconds of CPU
//! per frame and no GPU work at all. SDL's 2D renderer scales the picture
//! with the system's own graphics drivers.
//!
//! The player's ROM is read from `rom/` beside the app (or `--rom PATH`);
//! battery saves go to `saves/`, screenshots to `screenshots/`, settings to
//! `settings.ini`.
//!
//! Keys: arrows/WASD, X/K = A, Z/J = B, Enter = Start, Backspace/right
//! Shift = Select; F11 or Alt+Enter fullscreen, F1 scaling filter, F2 LCD
//! colours, F3 frame blending, F4 fit, F5 save state, F7 load it, F12
//! screenshot, P pause, hold Tab to fast-forward. Gamepads work as the
//! console's pad.

// A window, not a console, on Windows (errors go to a message box).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod input;
mod sdl;
mod settings;

use sdl3_sys::everything::*;
use settings::{Settings, Speed};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tcg2_core::{FRAME_RATE, ROM_SHA1, ROM_SIZE, SCREEN_HEIGHT, SCREEN_WIDTH};
use tcg2_kernel::Kernel;
use tcg2_render::{Filter, Fit, Picture};

const TITLE: &str = "Pokémon Card GB2";
const SAMPLE_RATE: u32 = 48_000;
/// Frames run at most per wake-up when behind (then the clock resyncs).
const MAX_CATCH_UP: u32 = 5;
const FAST_FORWARD: u32 = 4;
/// Battery RAM is written this long after the game last changed it.
const SAVE_DELAY: Duration = Duration::from_millis(750);

struct Options {
    rom: Option<PathBuf>,
    exit_after_frames: Option<u64>,
    screenshot: Option<PathBuf>,
    settings: Vec<(String, String)>,
    stats: bool,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        rom: None,
        exit_after_frames: None,
        screenshot: None,
        settings: Vec::new(),
        stats: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--rom" => o.rom = Some(value()?.into()),
            "--exit-after-frames" => {
                o.exit_after_frames = Some(value()?.parse().map_err(|_| "bad frame count")?)
            }
            "--screenshot" => o.screenshot = Some(value()?.into()),
            "--set" => {
                let v = value()?;
                let (k, val) = v.split_once('=').ok_or("--set KEY=VALUE")?;
                o.settings
                    .push((k.trim().to_string(), val.trim().to_string()));
            }
            "--stats" => o.stats = true,
            "--version" => {
                println!("{TITLE} {}", version());
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "{TITLE}\n\nusage: pokemon-tcg2 [--rom PATH] [--set KEY=VALUE]... [--stats]\n\
                     \nThe ROM is looked for in rom/ beside the app. Settings (settings.ini):\n\
                     filter=sharp|nearest|smooth fit=aspect|integer|stretch lcd_colours=BOOL\n\
                     frame_blending=BOOL fullscreen=BOOL volume=0-100 speed=exact|display\n\
                     max_fps=N pause_hidden=BOOL window=WxH"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument {arg:?} (see --help)")),
        }
    }
    Ok(o)
}

/// The release tag the build was made for (`TCG2_RELEASE_VERSION`), or the
/// crate's version.
fn version() -> &'static str {
    option_env!("TCG2_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// The player's ROM: `--rom`, else the first file in `rom/` (beside the app
/// or the working directory) that is the one the kernel was translated from.
fn load_rom(home: &Path, explicit: Option<&Path>) -> Result<Vec<u8>, String> {
    if let Some(path) = explicit {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        tcg2_core::verify_rom(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok(bytes);
    }
    let dirs = [home.join("rom"), PathBuf::from("rom")];
    for dir in &dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() != ROM_SIZE as u64 {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&path)
                && tcg2_core::verify_rom(&bytes).is_ok()
            {
                return Ok(bytes);
            }
        }
    }
    let _ = std::fs::create_dir_all(&dirs[0]);
    Err(format!(
        "No game ROM found.\n\nPut your own dump of \"Pokémon Card GB2 - GR Dan Sanjou! (J)\" \
         (2 MiB, SHA-1 {ROM_SHA1}) in\n{}\nand start the game again.",
        dirs[0].display()
    ))
}

fn write_png(path: &Path, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        SCREEN_WIDTH as u32,
        SCREEN_HEIGHT as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes battery RAM atomically (a temporary file, then a rename).
fn write_save(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = path.with_extension("sav.tmp");
    let result = std::fs::write(&temp, bytes).and_then(|_| std::fs::rename(&temp, path));
    if let Err(e) = result {
        eprintln!("pokemon-tcg2: saving {}: {e}", path.display());
    }
}

/// The game's frame rate: the console's own, or with `speed = display` the
/// display's refresh divided by the whole number that brings it within 1.5%
/// of the console's (60 Hz, 120 Hz, 240 Hz: every frame shown equally often).
fn game_rate(speed: Speed, refresh: Option<f32>) -> f64 {
    if let (Speed::Display, Some(hz)) = (speed, refresh) {
        let hz = hz as f64;
        let n = (hz / FRAME_RATE).round().max(1.0);
        if ((hz / n) / FRAME_RATE - 1.0).abs() < 0.015 {
            return hz / n;
        }
    }
    FRAME_RATE
}

/// The window's renderer and the picture it shows.
struct Display {
    renderer: sdl::Renderer,
    picture: Picture,
}

impl Display {
    fn new(window: &sdl::Window) -> Result<Self, String> {
        Ok(Self {
            renderer: sdl::Renderer::new(window, SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32)?,
            picture: Picture::default(),
        })
    }

    /// Prepares the picture for the current look (only when it or the look
    /// changed).
    fn update(&mut self, settings: &Settings) {
        let rgba = self.picture.compose(&settings.look);
        self.renderer.upload(rgba, SCREEN_WIDTH as i32 * 4);
    }

    /// Draws and presents; false if the renderer could not take a frame now.
    fn draw(&mut self, settings: &Settings) -> bool {
        self.renderer.set_scale_mode(match settings.look.filter {
            Filter::Sharp => SDL_SCALEMODE_PIXELART,
            Filter::Nearest => SDL_SCALEMODE_NEAREST,
            Filter::Smooth => SDL_SCALEMODE_LINEAR,
        });
        let rect = settings.look.fit.viewport(self.renderer.output_size());
        self.renderer.present(rect)
    }
}

/// Voluntary context switches so far (Linux; elsewhere 0): each is a sleep
/// and a wake-up of the main thread.
fn wakeups() -> u64 {
    #[cfg(target_os = "linux")]
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        return status
            .lines()
            .find(|l| l.starts_with("voluntary_ctxt_switches"))
            .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
            .unwrap_or(0);
    }
    0
}

/// Process CPU time in seconds (Linux; elsewhere 0).
fn cpu_seconds() -> f64 {
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
        let fields: Vec<&str> = stat
            .rsplit_once(')')
            .map(|(_, r)| r)
            .unwrap_or("")
            .split_whitespace()
            .collect();
        if fields.len() > 13 {
            let ticks =
                fields[11].parse::<f64>().unwrap_or(0.0) + fields[12].parse::<f64>().unwrap_or(0.0);
            return ticks / 100.0;
        }
    }
    0.0
}

fn main() {
    let options = match options() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pokemon-tcg2: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(options) {
        eprintln!("pokemon-tcg2: {e}");
        // TCG2_NO_DIALOG: report on stderr only (unattended runs, tests).
        if std::env::var_os("TCG2_NO_DIALOG").is_none() {
            sdl::message(None, TITLE, &e);
        }
        std::process::exit(1);
    }
}

fn run(options: Options) -> Result<(), String> {
    let home = settings::home();
    let settings_path = home.join("settings.ini");
    let mut settings = Settings::load(&settings_path);
    for (key, value) in &options.settings {
        if !settings.set(key, value) {
            return Err(format!("unknown setting {key}={value}"));
        }
    }
    let sdl = sdl::Sdl::init()?;
    let rom = load_rom(&home, options.rom.as_deref())?;
    let mut kernel = Kernel::new(rom, SAMPLE_RATE).map_err(|e| e.to_string())?;
    let save_path = home.join("saves").join("poketcg2.sav");
    if let Ok(bytes) = std::fs::read(&save_path) {
        kernel.load_sram(&bytes);
    }

    let window = sdl::Window::new(&sdl, TITLE, settings.window.0, settings.window.1)?;
    if settings.fullscreen {
        window.set_fullscreen(true);
    }
    let mut display = Display::new(&window)?;
    let mut audio = audio::Audio::open(SAMPLE_RATE, FRAME_RATE);
    let mut input = input::Input::default();

    let period = |settings: &Settings, window: &sdl::Window| {
        Duration::from_secs_f64(1.0 / game_rate(settings.speed, window.refresh_rate()))
    };
    let mut frame_period = period(&settings, &window);
    let started = Instant::now();
    let cpu_started = cpu_seconds();
    let wakeups_started = wakeups();
    let mut next_frame = Instant::now();
    let mut picture_pending = true;
    let mut redraw = true;
    // The look's colours changed: the picture is prepared again.
    let mut recompose = false;
    let mut last_present = Instant::now() - Duration::from_secs(1);
    let mut presented = 0u64;
    let mut paused = false;
    let mut sram_dirty_at: Option<Instant> = None;
    let mut samples = Vec::with_capacity(4096);
    let mut quit = false;

    'main: loop {
        // Sleep until the next frame is due or something happens.
        let hidden_pause = settings.pause_hidden && window.is_occluded();
        let now = Instant::now();
        // SAFETY: SDL is initialised; the event is written by SDL.
        let mut event: SDL_Event = unsafe { std::mem::zeroed() };
        let mut got = if paused || hidden_pause {
            // Nothing to run: sleep until something happens (a key, the
            // window shown again), with no periodic wake-ups at all.
            unsafe { SDL_WaitEvent(&mut event) }
        } else {
            let timeout_ms = next_frame
                .saturating_duration_since(now)
                .as_micros()
                .div_ceil(1000) as i32;
            timeout_ms > 0 && unsafe { SDL_WaitEventTimeout(&mut event, timeout_ms) }
        };
        loop {
            if got {
                let kind = SDL_EventType(unsafe { event.r#type });
                match kind {
                    SDL_EVENT_QUIT | SDL_EVENT_WINDOW_CLOSE_REQUESTED => quit = true,
                    SDL_EVENT_WINDOW_PIXEL_SIZE_CHANGED
                    | SDL_EVENT_WINDOW_RESIZED
                    | SDL_EVENT_WINDOW_EXPOSED
                    | SDL_EVENT_WINDOW_RESTORED
                    | SDL_EVENT_WINDOW_SHOWN => redraw = true,
                    SDL_EVENT_WINDOW_DISPLAY_CHANGED => frame_period = period(&settings, &window),
                    SDL_EVENT_GAMEPAD_ADDED => input.gamepad_added(unsafe { event.gdevice.which }),
                    SDL_EVENT_GAMEPAD_REMOVED => {
                        input.gamepad_removed(unsafe { event.gdevice.which })
                    }
                    SDL_EVENT_KEY_DOWN => {
                        let key = unsafe { event.key };
                        if !key.repeat {
                            let alt = key.r#mod & SDL_KMOD_ALT != SDL_Keymod(0);
                            match key.key {
                                SDLK_F11 => {
                                    settings.fullscreen = !window.is_fullscreen();
                                    window.set_fullscreen(settings.fullscreen);
                                }
                                SDLK_RETURN if alt => {
                                    settings.fullscreen = !window.is_fullscreen();
                                    window.set_fullscreen(settings.fullscreen);
                                }
                                SDLK_ESCAPE if window.is_fullscreen() => {
                                    settings.fullscreen = false;
                                    window.set_fullscreen(false);
                                }
                                SDLK_F1 => {
                                    let i = Filter::ALL
                                        .iter()
                                        .position(|&f| f == settings.look.filter)
                                        .unwrap_or(0);
                                    settings.look.filter = Filter::ALL[(i + 1) % Filter::ALL.len()];
                                    window.set_title(&format!(
                                        "{TITLE} - scaling: {}",
                                        settings.look.filter.name()
                                    ));
                                    redraw = true;
                                }
                                SDLK_F2 => {
                                    settings.look.lcd_colours = !settings.look.lcd_colours;
                                    let state = if settings.look.lcd_colours {
                                        "LCD"
                                    } else {
                                        "raw"
                                    };
                                    window.set_title(&format!("{TITLE} - colours: {state}"));
                                    recompose = true;
                                }
                                SDLK_F3 => {
                                    settings.look.frame_blending = !settings.look.frame_blending;
                                    let state = if settings.look.frame_blending {
                                        "on"
                                    } else {
                                        "off"
                                    };
                                    window.set_title(&format!("{TITLE} - frame blending: {state}"));
                                    recompose = true;
                                }
                                SDLK_F4 => {
                                    let i = Fit::ALL
                                        .iter()
                                        .position(|&f| f == settings.look.fit)
                                        .unwrap_or(0);
                                    settings.look.fit = Fit::ALL[(i + 1) % Fit::ALL.len()];
                                    window.set_title(&format!(
                                        "{TITLE} - fit: {}",
                                        settings.look.fit.name()
                                    ));
                                    redraw = true;
                                }
                                SDLK_F5 => {
                                    let path = home.join("saves").join("quick.state");
                                    let _ = std::fs::create_dir_all(home.join("saves"));
                                    let result = std::fs::write(&path, kernel.machine.save_state());
                                    window.set_title(&match result {
                                        Ok(()) => format!("{TITLE} - state saved (F7 loads it)"),
                                        Err(e) => format!("{TITLE} - state not saved: {e}"),
                                    });
                                }
                                SDLK_F7 => {
                                    let path = home.join("saves").join("quick.state");
                                    let result = std::fs::read(&path)
                                        .map_err(|e| e.to_string())
                                        .and_then(|bytes| {
                                            kernel
                                                .machine
                                                .load_state(&bytes)
                                                .map_err(|e| e.to_string())
                                        });
                                    match result {
                                        Ok(()) => {
                                            window.set_title(&format!("{TITLE} - state loaded"));
                                            picture_pending = true;
                                            if let Some(audio) = &mut audio {
                                                audio.stop();
                                            }
                                            next_frame = Instant::now();
                                        }
                                        Err(e) => window
                                            .set_title(&format!("{TITLE} - no state loaded: {e}")),
                                    }
                                }
                                SDLK_F12 => {
                                    let stamp = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis())
                                        .unwrap_or(0);
                                    let path =
                                        home.join("screenshots").join(format!("tcg2-{stamp}.png"));
                                    if let Err(e) = write_png(&path, kernel.framebuffer()) {
                                        eprintln!("pokemon-tcg2: {e}");
                                    }
                                }
                                SDLK_P | SDLK_PAUSE => {
                                    paused = !paused;
                                    window.set_title(if paused {
                                        "Pokémon Card GB2 (paused)"
                                    } else {
                                        TITLE
                                    });
                                    if paused && let Some(audio) = &mut audio {
                                        audio.stop();
                                    }
                                    next_frame = Instant::now();
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            // SAFETY: as above.
            got = unsafe { SDL_PollEvent(&mut event) };
            if !got {
                break;
            }
        }
        if quit {
            break 'main;
        }

        // Run the frames that are due.
        let hidden_pause = settings.pause_hidden && window.is_occluded();
        if paused || hidden_pause {
            next_frame = Instant::now();
            if hidden_pause && let Some(audio) = &mut audio {
                audio.stop();
            }
        } else {
            let fast = {
                let mut count = 0;
                // SAFETY: SDL owns the array.
                let keys = unsafe { SDL_GetKeyboardState(&mut count) };
                window.has_focus()
                    && !keys.is_null()
                    && SDL_SCANCODE_TAB.0 < count
                    && unsafe { *keys.add(SDL_SCANCODE_TAB.0 as usize) }
            };
            let mut ran = 0;
            while Instant::now() >= next_frame {
                kernel.set_buttons(input.buttons(window.has_focus()));
                let bursts = if fast { FAST_FORWARD } else { 1 };
                for burst in 0..bursts {
                    if let Err(fault) = kernel.run_frame() {
                        // Keep what the game saved before it stopped.
                        if sram_dirty_at.is_some() || kernel.take_sram_dirty() {
                            write_save(&save_path, kernel.sram());
                        }
                        return Err(fault.to_string());
                    }
                    samples.clear();
                    kernel.drain_audio(&mut samples);
                    if burst == 0
                        && let Some(audio) = &mut audio
                    {
                        audio.volume = settings.volume as f32 / 100.0;
                        audio.queue(&samples, kernel.machine.apu.silent);
                    }
                }
                if kernel.take_picture_changed() {
                    picture_pending = true;
                }
                if kernel.take_sram_dirty() {
                    sram_dirty_at = Some(Instant::now());
                }
                next_frame += frame_period;
                ran += 1;
                if ran >= MAX_CATCH_UP {
                    if Instant::now() > next_frame {
                        next_frame = Instant::now() + frame_period;
                    }
                    break;
                }
                if options
                    .exit_after_frames
                    .is_some_and(|n| kernel.machine.frames >= n)
                {
                    break 'main;
                }
            }
        }

        // Present only something new, and no more often than the cap.
        let cap_ok = settings.max_fps == 0
            || last_present.elapsed().as_secs_f64() >= 1.0 / settings.max_fps as f64 - 0.001;
        if (picture_pending || recompose || redraw) && cap_ok && !window.is_occluded() {
            if picture_pending {
                display.picture.push(kernel.framebuffer());
            }
            if picture_pending || recompose {
                display.update(&settings);
                picture_pending = false;
                recompose = false;
            }
            if display.draw(&settings) {
                redraw = false;
                last_present = Instant::now();
                presented += 1;
            }
        }

        if sram_dirty_at.is_some_and(|at| at.elapsed() >= SAVE_DELAY) {
            write_save(&save_path, kernel.sram());
            sram_dirty_at = None;
        }
    }

    if sram_dirty_at.is_some() || kernel.take_sram_dirty() {
        write_save(&save_path, kernel.sram());
    }
    if let Some(path) = &options.screenshot {
        write_png(path, kernel.framebuffer())?;
    }
    if !window.is_fullscreen() {
        let (mut w, mut h) = (0, 0);
        // SAFETY: live window, valid out pointers.
        unsafe { SDL_GetWindowSize(window.raw, &mut w, &mut h) };
        if w > 0 && h > 0 {
            settings.window = (w, h);
        }
    }
    settings.save(&settings_path);
    if options.stats {
        let wall = started.elapsed().as_secs_f64();
        let cpu = cpu_seconds() - cpu_started;
        println!(
            "{} frames in {wall:.1} s: {presented} pictures presented, CPU {cpu:.2} s ({:.1}% of one core), \
             {:.0} main-thread wake-ups per second",
            kernel.machine.frames,
            100.0 * cpu / wall.max(1e-9),
            (wakeups() - wakeups_started) as f64 / wall.max(1e-9)
        );
    }
    drop(display);
    drop(window);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_speed_locks_to_whole_multiples_of_sixty() {
        assert_eq!(game_rate(Speed::Display, Some(60.0)), 60.0);
        assert_eq!(game_rate(Speed::Display, Some(120.0)), 60.0);
        assert!((game_rate(Speed::Display, Some(239.76)) - 59.94).abs() < 1e-4);
        assert_eq!(game_rate(Speed::Display, Some(144.0)), FRAME_RATE);
        assert_eq!(game_rate(Speed::Display, Some(75.0)), FRAME_RATE);
        assert_eq!(game_rate(Speed::Exact, Some(60.0)), FRAME_RATE);
        assert_eq!(game_rate(Speed::Display, None), FRAME_RATE);
    }
}
