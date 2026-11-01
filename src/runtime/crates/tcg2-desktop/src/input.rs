//! Keyboard and gamepads to the console's eight buttons, read once per game
//! frame from SDL's state (no per-event bookkeeping).

use sdl3_sys::everything::*;
use tcg2_core::buttons;

/// Default keys (SDL scancodes, physical positions): arrows or WASD for the
/// pad, X/K for A, Z/J for B, Enter for Start, Backspace/right Shift for
/// Select.
const KEYS: &[(SDL_Scancode, u8)] = &[
    (SDL_SCANCODE_UP, buttons::UP),
    (SDL_SCANCODE_DOWN, buttons::DOWN),
    (SDL_SCANCODE_LEFT, buttons::LEFT),
    (SDL_SCANCODE_RIGHT, buttons::RIGHT),
    (SDL_SCANCODE_W, buttons::UP),
    (SDL_SCANCODE_S, buttons::DOWN),
    (SDL_SCANCODE_A, buttons::LEFT),
    (SDL_SCANCODE_D, buttons::RIGHT),
    (SDL_SCANCODE_X, buttons::A),
    (SDL_SCANCODE_K, buttons::A),
    (SDL_SCANCODE_Z, buttons::B),
    (SDL_SCANCODE_J, buttons::B),
    (SDL_SCANCODE_RETURN, buttons::START),
    (SDL_SCANCODE_KP_ENTER, buttons::START),
    (SDL_SCANCODE_BACKSPACE, buttons::SELECT),
    (SDL_SCANCODE_RSHIFT, buttons::SELECT),
];

/// Gamepad buttons by position: east/north for A, south/west for B (the
/// console's A is the right-hand button).
const PAD: &[(SDL_GamepadButton, u8)] = &[
    (SDL_GAMEPAD_BUTTON_DPAD_UP, buttons::UP),
    (SDL_GAMEPAD_BUTTON_DPAD_DOWN, buttons::DOWN),
    (SDL_GAMEPAD_BUTTON_DPAD_LEFT, buttons::LEFT),
    (SDL_GAMEPAD_BUTTON_DPAD_RIGHT, buttons::RIGHT),
    (SDL_GAMEPAD_BUTTON_EAST, buttons::A),
    (SDL_GAMEPAD_BUTTON_NORTH, buttons::A),
    (SDL_GAMEPAD_BUTTON_SOUTH, buttons::B),
    (SDL_GAMEPAD_BUTTON_WEST, buttons::B),
    (SDL_GAMEPAD_BUTTON_START, buttons::START),
    (SDL_GAMEPAD_BUTTON_BACK, buttons::SELECT),
];

const STICK_THRESHOLD: i16 = 16000;

#[derive(Default)]
pub struct Input {
    pads: Vec<(SDL_JoystickID, *mut SDL_Gamepad)>,
}

impl Input {
    pub fn gamepad_added(&mut self, id: SDL_JoystickID) {
        // SAFETY: SDL reported the id as a gamepad.
        let pad = unsafe { SDL_OpenGamepad(id) };
        if !pad.is_null() {
            self.pads.push((id, pad));
        }
    }

    pub fn gamepad_removed(&mut self, id: SDL_JoystickID) {
        self.pads.retain(|&(pad_id, pad)| {
            if pad_id == id {
                // SAFETY: opened by SDL_OpenGamepad and closed once.
                unsafe { SDL_CloseGamepad(pad) };
                false
            } else {
                true
            }
        });
    }

    pub fn has_gamepad(&self) -> bool {
        !self.pads.is_empty()
    }

    /// The buttons held now.
    pub fn buttons(&self, keyboard_focus: bool) -> u8 {
        let mut held = 0;
        if keyboard_focus {
            let mut count = 0;
            // SAFETY: SDL owns the array, valid until SDL_Quit.
            let keys = unsafe { SDL_GetKeyboardState(&mut count) };
            if !keys.is_null() {
                // SAFETY: `count` entries.
                let keys = unsafe { std::slice::from_raw_parts(keys, count.max(0) as usize) };
                for &(code, button) in KEYS {
                    if keys.get(code.0 as usize).copied().unwrap_or(false) {
                        held |= button;
                    }
                }
            }
        }
        for &(_, pad) in &self.pads {
            // SAFETY: an open gamepad.
            unsafe {
                for &(b, button) in PAD {
                    if SDL_GetGamepadButton(pad, b) {
                        held |= button;
                    }
                }
                let x = SDL_GetGamepadAxis(pad, SDL_GAMEPAD_AXIS_LEFTX);
                let y = SDL_GetGamepadAxis(pad, SDL_GAMEPAD_AXIS_LEFTY);
                if x <= -STICK_THRESHOLD {
                    held |= buttons::LEFT;
                }
                if x >= STICK_THRESHOLD {
                    held |= buttons::RIGHT;
                }
                if y <= -STICK_THRESHOLD {
                    held |= buttons::UP;
                }
                if y >= STICK_THRESHOLD {
                    held |= buttons::DOWN;
                }
            }
        }
        // The console's pad cannot report opposite directions at once.
        if held & (buttons::LEFT | buttons::RIGHT) == buttons::LEFT | buttons::RIGHT {
            held &= !(buttons::LEFT | buttons::RIGHT);
        }
        if held & (buttons::UP | buttons::DOWN) == buttons::UP | buttons::DOWN {
            held &= !(buttons::UP | buttons::DOWN);
        }
        held
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        for &(_, pad) in &self.pads {
            // SAFETY: opened by SDL_OpenGamepad and closed once.
            unsafe { SDL_CloseGamepad(pad) };
        }
    }
}
