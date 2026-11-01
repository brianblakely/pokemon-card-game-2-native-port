//! Android. SDL's Java activity (`src/runtime/android`) loads this library
//! as `libmain.so` and calls [`SDL_main`] on a thread of its own; from there
//! the app runs as on the desktop, except that:
//!
//! - its files (`rom/`, `saves/`, `screenshots/`, `settings.ini`) are in
//!   the app's folder of the device's shared storage,
//!   `Android/data/<package>/files/`, which a computer reaches over USB;
//! - without a ROM there, the player picks theirs with the system's
//!   document picker, and a copy is kept in `rom/`;
//! - the console's buttons are drawn for touch (`touch.rs`);
//! - messages go to the system log (logcat's `SDL/APP`).

use sdl3_sys::everything::*;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tcg2_core::{ROM_SHA1, ROM_SIZE};

/// SDL's activity runs this with the arguments it was started with: none,
/// or the desktop's command line from the intent's `args` extra (tests).
///
/// # Safety
///
/// `argv` holds `argc` NUL-terminated strings, the program's name first,
/// as SDL passes them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    std::panic::set_hook(Box::new(|info| log(&format!("panic: {info}"))));
    // SAFETY: as the caller promises.
    let args: Vec<String> = (1..argc.max(1) as usize)
        .map(|n| {
            unsafe { CStr::from_ptr(*argv.add(n)) }
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    crate::start(args.into_iter()).into()
}

/// Writes `pokemon-tcg2: message` to the system log.
pub fn log(message: &str) {
    let line = CString::new(format!("pokemon-tcg2: {message}")).unwrap_or_default();
    // SAFETY: a "%s" format with one NUL-terminated string.
    unsafe { SDL_Log(c"%s".as_ptr(), line.as_ptr()) };
}

/// The app's folder in shared storage, or its private one when shared
/// storage cannot be written.
pub fn files_dir() -> PathBuf {
    // SAFETY: SDL's activity has set up its Java side before SDL_main; the
    // strings belong to SDL.
    unsafe {
        let shared = SDL_GetAndroidExternalStorageState() & SDL_ANDROID_EXTERNAL_STORAGE_WRITE != 0;
        let path = if shared {
            SDL_GetAndroidExternalStoragePath()
        } else {
            std::ptr::null()
        };
        let path = if path.is_null() {
            SDL_GetAndroidInternalStoragePath()
        } else {
            path
        };
        if path.is_null() {
            return PathBuf::from(".");
        }
        PathBuf::from(CStr::from_ptr(path).to_string_lossy().into_owned())
    }
}

/// Asks the player for their ROM with the system's document picker, and
/// keeps a copy in `home`'s `rom/`, where the next start finds it. None if
/// they chose to quit instead.
pub fn import_rom(home: &Path) -> Option<Vec<u8>> {
    let copy = home.join("rom").join("poketcg2.gbc");
    log(&format!(
        "no ROM in {}: asking for one",
        copy.parent()?.display()
    ));
    let ask = format!(
        "Choose your own dump of \"Pokémon Card GB2 - GR Dan Sanjou! (J)\" (2 MiB, SHA-1 {ROM_SHA1}). \
         The game reads its graphics, text and music from it; a copy is kept in the app's folder."
    );
    let mut message = ask.clone();
    loop {
        if !choose(&message) {
            log("no ROM chosen: quitting");
            return None;
        }
        let uri = match pick() {
            Picked::Document(uri) => uri,
            Picked::Cancelled => {
                message = ask.clone();
                continue;
            }
            Picked::Quit => return None,
        };
        match read_rom(&uri) {
            Ok(rom) => {
                let kept = std::fs::create_dir_all(home.join("rom"))
                    .and_then(|()| std::fs::write(&copy, &rom));
                match kept {
                    Ok(()) => log(&format!("ROM {uri} kept as {}", copy.display())),
                    Err(e) => log(&format!(
                        "ROM {uri} not kept ({}: {e}); asking again next time",
                        copy.display()
                    )),
                }
                return Some(rom);
            }
            Err(e) => {
                log(&format!("{uri}: {e}"));
                message = format!("That file is not the game's ROM: {e}.\n\n{ask}");
            }
        }
    }
}

/// Shows `message` with Choose ROM and Quit; true for Choose ROM.
fn choose(message: &str) -> bool {
    let message = CString::new(message).unwrap_or_default();
    let title = CString::new(crate::TITLE).unwrap_or_default();
    let buttons = [
        SDL_MessageBoxButtonData {
            flags: SDL_MESSAGEBOX_BUTTON_ESCAPEKEY_DEFAULT,
            buttonID: 0,
            text: c"Quit".as_ptr(),
        },
        SDL_MessageBoxButtonData {
            flags: SDL_MESSAGEBOX_BUTTON_RETURNKEY_DEFAULT,
            buttonID: 1,
            text: c"Choose ROM".as_ptr(),
        },
    ];
    let data = SDL_MessageBoxData {
        flags: SDL_MESSAGEBOX_INFORMATION,
        window: std::ptr::null_mut(),
        title: title.as_ptr(),
        message: message.as_ptr(),
        numbuttons: buttons.len() as c_int,
        buttons: buttons.as_ptr(),
        colorScheme: std::ptr::null(),
    };
    let mut chosen = 0;
    // SAFETY: the data and its strings outlive the call, which returns when
    // the player answers.
    unsafe { SDL_ShowMessageBox(&data, &mut chosen) && chosen == 1 }
}

enum Picked {
    Document(String),
    Cancelled,
    /// The activity is ending.
    Quit,
}

/// The picker's answer, set on Android's UI thread: a document's URI, or
/// None if the player went back.
static ANSWER: Mutex<Option<Result<Option<String>, String>>> = Mutex::new(None);

unsafe extern "C" fn answered(_: *mut c_void, files: *const *const c_char, _: c_int) {
    // SAFETY: SDL passes NULL on an error, else a NULL-terminated list of
    // NUL-terminated strings.
    let answer = unsafe {
        if files.is_null() {
            Err(crate::sdl::error())
        } else if (*files).is_null() {
            Ok(None)
        } else {
            Ok(Some(CStr::from_ptr(*files).to_string_lossy().into_owned()))
        }
    };
    *ANSWER.lock().unwrap_or_else(|e| e.into_inner()) = Some(answer);
    // Wakes the SDL thread waiting for the answer.
    let mut event = SDL_Event::default();
    event.r#type = SDL_EVENT_USER.0;
    // SAFETY: SDL's event queue may be used from any thread.
    unsafe { SDL_PushEvent(&mut event) };
}

/// Shows the system's document picker and waits, asleep, for the answer.
fn pick() -> Picked {
    *ANSWER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // SAFETY: SDL is initialised; the callback is a plain function.
    unsafe {
        SDL_ShowOpenFileDialog(
            Some(answered),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            false,
        )
    };
    loop {
        if let Some(answer) = ANSWER.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return match answer {
                Ok(Some(uri)) => Picked::Document(uri),
                Ok(None) => Picked::Cancelled,
                Err(e) => {
                    log(&format!("no document picker: {e}"));
                    Picked::Cancelled
                }
            };
        }
        let mut event = SDL_Event::default();
        // SAFETY: SDL is initialised; the event it writes starts with its
        // type.
        if unsafe { SDL_WaitEvent(&mut event) && SDL_EventType(event.r#type) == SDL_EVENT_QUIT } {
            return Picked::Quit;
        }
    }
}

/// Reads the document at `uri` (a content:// URI) if it is the ROM: at
/// most a byte more than a ROM, whatever the player picked.
fn read_rom(uri: &str) -> Result<Vec<u8>, String> {
    let name = CString::new(uri).map_err(|e| e.to_string())?;
    let mut rom = vec![0u8; ROM_SIZE + 1];
    let mut filled = 0;
    // SAFETY: SDL reads the document through Android's content resolver
    // into the buffer's unfilled part; the stream is closed once.
    unsafe {
        let stream = SDL_IOFromFile(name.as_ptr(), c"rb".as_ptr());
        if stream.is_null() {
            return Err(crate::sdl::error());
        }
        while filled < rom.len() {
            let read = SDL_ReadIO(
                stream,
                rom[filled..].as_mut_ptr().cast(),
                rom.len() - filled,
            );
            if read == 0 {
                break;
            }
            filled += read;
        }
        SDL_CloseIO(stream);
    }
    rom.truncate(filled);
    tcg2_core::verify_rom(&rom).map_err(|e| e.to_string())?;
    Ok(rom)
}
