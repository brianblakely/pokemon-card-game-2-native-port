//! `tcg2-libretro`: runs the ROM in a reference emulator (a libretro Game
//! Boy core such as SameBoy or gambatte) with the same scripted input as
//! `tcg2-headless`, to compare pictures and the CPU cost of emulation with
//! the kernel's.
//!
//! ```text
//! tcg2-libretro --core $(nix build --print-out-paths nixpkgs#libretro.sameboy)/lib/retroarch/cores/sameboy_libretro.so \
//!     --rom rom/poketcg2.gbc --frames 2400 --shots 100:build/ref
//! ```
//!
//! Colour correction is switched off so pictures compare with the raw
//! colours the kernel draws.

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tcg2_core::buttons;
use tcg2_kernel::script::Script;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;

#[repr(C)]
struct GameInfo {
    path: *const c_char,
    data: *const c_void,
    size: usize,
    meta: *const c_char,
}

#[repr(C)]
struct Variable {
    key: *const c_char,
    value: *const c_char,
}

struct Frame {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    format: u32,
    buttons: u8,
}

static AUDIO: Mutex<Vec<i16>> = Mutex::new(Vec::new());
static FRAME: Mutex<Frame> = Mutex::new(Frame {
    pixels: Vec::new(),
    width: 0,
    height: 0,
    format: 0,
    buttons: 0,
});
static OPTIONS: &[(&str, &str)] = &[
    ("sameboy_color_correction_mode", "off"),
    ("sameboy_model", "Auto"),
    ("gambatte_gbc_color_correction", "disabled"),
    ("gambatte_gb_colorization", "disabled"),
    ("mgba_color_correction", "OFF"),
];

unsafe extern "C" fn environment(cmd: c_uint, data: *mut c_void) -> bool {
    unsafe {
        match cmd {
            // GET_CAN_DUPE
            3 => {
                *(data as *mut bool) = true;
                true
            }
            // GET_SYSTEM_DIRECTORY, GET_SAVE_DIRECTORY
            9 | 31 => {
                static DIR: &CStr = c".";
                *(data as *mut *const c_char) = DIR.as_ptr();
                true
            }
            // SET_PIXEL_FORMAT
            10 => {
                FRAME.lock().unwrap().format = *(data as *const u32);
                true
            }
            // GET_VARIABLE
            15 => {
                let var = &mut *(data as *mut Variable);
                let key = CStr::from_ptr(var.key).to_string_lossy();
                for (k, v) in OPTIONS {
                    if *k == key {
                        // Leaked once per key: the core keeps the pointer.
                        var.value = CString::new(*v).unwrap().into_raw();
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

unsafe extern "C" fn video(data: *const c_void, width: c_uint, height: c_uint, pitch: usize) {
    if data.is_null() {
        return;
    }
    let mut guard = FRAME.lock().unwrap();
    let f = &mut *guard;
    f.width = width as usize;
    f.height = height as usize;
    let bpp = if f.format == 1 { 4 } else { 2 };
    let size = f.width * f.height * 4;
    f.pixels.resize(size, 0);
    for y in 0..f.height {
        // SAFETY: the core gives `height` rows of `pitch` bytes.
        let row = unsafe {
            std::slice::from_raw_parts((data as *const u8).add(y * pitch), f.width * bpp)
        };
        for x in 0..f.width {
            let rgba = if bpp == 4 {
                [row[x * 4 + 2], row[x * 4 + 1], row[x * 4], 0xff]
            } else {
                let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]);
                let (r, g, b) = ((v >> 11) & 31, (v >> 5) & 63, v & 31);
                [
                    ((r << 3) | (r >> 2)) as u8,
                    ((g << 2) | (g >> 4)) as u8,
                    ((b << 3) | (b >> 2)) as u8,
                    0xff,
                ]
            };
            let i = (y * f.width + x) * 4;
            f.pixels[i..i + 4].copy_from_slice(&rgba);
        }
    }
}

unsafe extern "C" fn audio_sample(l: i16, r: i16) {
    AUDIO.lock().unwrap().extend_from_slice(&[l, r]);
}
unsafe extern "C" fn audio_batch(data: *const i16, frames: usize) -> usize {
    // SAFETY: the core passes `frames` stereo frames.
    let samples = unsafe { std::slice::from_raw_parts(data, frames * 2) };
    AUDIO.lock().unwrap().extend_from_slice(samples);
    frames
}

#[repr(C)]
#[derive(Default)]
struct AvInfo {
    base_width: c_uint,
    base_height: c_uint,
    max_width: c_uint,
    max_height: c_uint,
    aspect: f32,
    fps: f64,
    sample_rate: f64,
}

fn write_wav(path: &Path, samples: &[i16], rate: u32) {
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}
unsafe extern "C" fn input_poll() {}
unsafe extern "C" fn input_state(port: c_uint, device: c_uint, _index: c_uint, id: c_uint) -> i16 {
    if port != 0 || device != 1 {
        return 0;
    }
    let held = FRAME.lock().unwrap().buttons;
    let bit = match id {
        0 => buttons::B,
        2 => buttons::SELECT,
        3 => buttons::START,
        4 => buttons::UP,
        5 => buttons::DOWN,
        6 => buttons::LEFT,
        7 => buttons::RIGHT,
        8 => buttons::A,
        _ => 0,
    };
    (held & bit != 0) as i16
}

struct Core {
    handle: *mut c_void,
}

impl Core {
    fn symbol<T: Copy>(&self, name: &str) -> T {
        let c = CString::new(name).unwrap();
        // SAFETY: a loaded library; the caller names the libretro type.
        unsafe {
            let p = dlsym(self.handle, c.as_ptr());
            assert!(!p.is_null(), "core has no {name}");
            std::mem::transmute_copy(&p)
        }
    }
}

fn write_png(path: &Path, rgba: &[u8], width: usize, height: usize) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

fn cpu_seconds() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .map(|(_, r)| r)
        .unwrap_or("")
        .split_whitespace()
        .collect();
    if fields.len() > 13 {
        (fields[11].parse::<f64>().unwrap_or(0.0) + fields[12].parse::<f64>().unwrap_or(0.0))
            / 100.0
    } else {
        0.0
    }
}

fn main() {
    let mut core_path: Option<PathBuf> = None;
    let mut rom_path = PathBuf::from("rom/poketcg2.gbc");
    let mut frames = 600u64;
    let mut script = Script::default();
    let mut shots: Option<(u64, PathBuf)> = None;
    let mut wav: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
        match arg.as_str() {
            "--core" => core_path = Some(value.into()),
            "--rom" => rom_path = value.into(),
            "--frames" => frames = value.parse().expect("frames"),
            "--press" => script
                .presses
                .push(value.parse().unwrap_or_else(|e: String| panic!("{e}"))),
            "--random" => {
                script.random = Some(value.parse().unwrap_or_else(|e: String| panic!("{e}")))
            }
            "--wav" => wav = Some(value.into()),
            "--shots" => {
                let (every, dir) = value.split_once(':').expect("--shots N:DIR");
                shots = Some((every.parse().expect("interval"), dir.into()));
            }
            _ => panic!("unknown argument {arg}"),
        }
    }
    let core_path = core_path.expect("--core PATH to a libretro core");
    let rom = std::fs::read(&rom_path).expect("ROM");
    let c = CString::new(core_path.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: loading a libretro core and calling its C API as specified.
    unsafe {
        let handle = dlopen(c.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            panic!("dlopen: {}", CStr::from_ptr(dlerror()).to_string_lossy());
        }
        let core = Core { handle };
        core.symbol::<unsafe extern "C" fn(unsafe extern "C" fn(c_uint, *mut c_void) -> bool)>(
            "retro_set_environment",
        )(environment);
        core.symbol::<unsafe extern "C" fn(unsafe extern "C" fn(*const c_void, c_uint, c_uint, usize))>("retro_set_video_refresh")(video);
        core.symbol::<unsafe extern "C" fn(unsafe extern "C" fn(i16, i16))>(
            "retro_set_audio_sample",
        )(audio_sample);
        core.symbol::<unsafe extern "C" fn(unsafe extern "C" fn(*const i16, usize) -> usize)>(
            "retro_set_audio_sample_batch",
        )(audio_batch);
        core.symbol::<unsafe extern "C" fn(unsafe extern "C" fn())>("retro_set_input_poll")(
            input_poll,
        );
        core.symbol::<unsafe extern "C" fn(unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint) -> i16)>("retro_set_input_state")(input_state);
        core.symbol::<unsafe extern "C" fn()>("retro_init")();
        let path = CString::new(rom_path.to_string_lossy().as_bytes()).unwrap();
        let info = GameInfo {
            path: path.as_ptr(),
            data: rom.as_ptr().cast(),
            size: rom.len(),
            meta: std::ptr::null(),
        };
        assert!(
            core.symbol::<unsafe extern "C" fn(*const GameInfo) -> bool>("retro_load_game")(&info),
            "the core refused the ROM"
        );
        let run = core.symbol::<unsafe extern "C" fn()>("retro_run");
        let mut av = AvInfo::default();
        core.symbol::<unsafe extern "C" fn(*mut AvInfo)>("retro_get_system_av_info")(&mut av);
        let started = Instant::now();
        let cpu_started = cpu_seconds();
        for frame in 0..frames {
            FRAME.lock().unwrap().buttons = script.buttons(frame);
            run();
            if let Some((every, dir)) = &shots
                && (frame + 1) % every == 0
            {
                let f = FRAME.lock().unwrap();
                write_png(
                    &dir.join(format!("{:06}.png", frame + 1)),
                    &f.pixels,
                    f.width,
                    f.height,
                );
            }
        }
        let wall = started.elapsed().as_secs_f64();
        let cpu = cpu_seconds() - cpu_started;
        if let Some(path) = &wav {
            write_wav(path, &AUDIO.lock().unwrap(), av.sample_rate.round() as u32);
        }
        let game_seconds = frames as f64 / tcg2_core::FRAME_RATE;
        println!(
            "{} : {frames} frames ({game_seconds:.1} s of game time) in {wall:.3} s wall, {cpu:.2} s CPU: {:.2} ms of host time per game second",
            core_path.file_name().unwrap().to_string_lossy(),
            1000.0 * wall / game_seconds
        );
    }
}
