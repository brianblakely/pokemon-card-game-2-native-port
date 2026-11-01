//! SDL3: initialisation, the window, and its 2D renderer.

use crate::touch::Rect;
use sdl3_sys::everything::*;
use std::ffi::{CStr, CString};
use std::ptr::{null, null_mut};

pub fn error() -> String {
    // SAFETY: SDL_GetError returns a valid, NUL-terminated thread-local string.
    unsafe { CStr::from_ptr(SDL_GetError()) }
        .to_string_lossy()
        .into_owned()
}

/// SDL itself: initialised on creation, shut down on drop.
pub struct Sdl;

impl Sdl {
    pub fn init() -> Result<Self, String> {
        // SAFETY: called once on the main thread before any other SDL call.
        unsafe {
            let name = CString::new("Pokémon Card GB2").unwrap();
            SDL_SetAppMetadata(name.as_ptr(), std::ptr::null(), std::ptr::null());
            // A 1024-frame device period (21 ms at 48 kHz): the audio thread
            // wakes about 47 times a second instead of SDL's default ~94.
            let hint = CString::new("SDL_AUDIO_DEVICE_SAMPLE_FRAMES").unwrap();
            let frames = CString::new("1024").unwrap();
            SDL_SetHint(hint.as_ptr(), frames.as_ptr());
            if SDL_Init(SDL_INIT_VIDEO | SDL_INIT_EVENTS | SDL_INIT_GAMEPAD | SDL_INIT_AUDIO) {
                Ok(Sdl)
            } else {
                Err(format!("SDL_Init: {}", error()))
            }
        }
    }
}

impl Drop for Sdl {
    fn drop(&mut self) {
        // SAFETY: every SDL object has been released by now.
        unsafe { SDL_Quit() };
    }
}

/// Shows a message box (with no window, or over `window`).
pub fn message(window: Option<&Window>, title: &str, text: &str) {
    let title = CString::new(title).unwrap_or_default();
    let text = CString::new(text).unwrap_or_default();
    // SAFETY: valid strings; a null window is allowed.
    unsafe {
        SDL_ShowSimpleMessageBox(
            SDL_MESSAGEBOX_ERROR,
            title.as_ptr(),
            text.as_ptr(),
            window.map(|w| w.raw).unwrap_or(null_mut()),
        );
    }
}

pub struct Window {
    pub raw: *mut SDL_Window,
}

impl Window {
    /// A resizable high-density window.
    pub fn new(_sdl: &Sdl, title: &str, width: i32, height: i32) -> Result<Self, String> {
        let title = CString::new(title).expect("title without NUL");
        let flags = SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY;
        // SAFETY: SDL is initialised; the title outlives the call.
        let raw = unsafe { SDL_CreateWindow(title.as_ptr(), width, height, flags) };
        if raw.is_null() {
            return Err(format!("SDL_CreateWindow: {}", error()));
        }
        Ok(Self { raw })
    }

    pub fn set_fullscreen(&self, fullscreen: bool) {
        // SAFETY: a live window, on SDL's main thread.
        unsafe { SDL_SetWindowFullscreen(self.raw, fullscreen) };
    }

    pub fn is_fullscreen(&self) -> bool {
        // SAFETY: a live window.
        unsafe { SDL_GetWindowFlags(self.raw) & SDL_WINDOW_FULLSCREEN != SDL_WindowFlags(0) }
    }

    /// Hidden or minimised: nothing drawn would be seen.
    pub fn is_occluded(&self) -> bool {
        // SAFETY: a live window.
        let flags = unsafe { SDL_GetWindowFlags(self.raw) };
        flags & (SDL_WINDOW_MINIMIZED | SDL_WINDOW_HIDDEN | SDL_WINDOW_OCCLUDED)
            != SDL_WindowFlags(0)
    }

    pub fn has_focus(&self) -> bool {
        // SAFETY: a live window.
        unsafe { SDL_GetWindowFlags(self.raw) & SDL_WINDOW_INPUT_FOCUS != SDL_WindowFlags(0) }
    }

    /// The display's refresh rate, if SDL knows it.
    pub fn refresh_rate(&self) -> Option<f32> {
        // SAFETY: live window; the mode belongs to SDL.
        unsafe {
            let display = SDL_GetDisplayForWindow(self.raw);
            let mode = SDL_GetCurrentDisplayMode(display);
            if mode.is_null() || (*mode).refresh_rate <= 0.0 {
                None
            } else {
                Some((*mode).refresh_rate)
            }
        }
    }

    pub fn set_title(&self, title: &str) {
        let title = CString::new(title).expect("title without NUL");
        // SAFETY: live window; SDL copies the title.
        unsafe { SDL_SetWindowTitle(self.raw, title.as_ptr()) };
    }

    /// The part of the window clear of display cutouts and the system's
    /// bars, in the pixels of a drawable `pixels` big.
    pub fn safe_area(&self, pixels: (u32, u32)) -> Rect {
        let mut safe = SDL_Rect::default();
        let (mut w, mut h) = (0, 0);
        // SAFETY: a live window, valid out pointers.
        let known = unsafe {
            SDL_GetWindowSize(self.raw, &mut w, &mut h)
                && SDL_GetWindowSafeArea(self.raw, &mut safe)
        };
        if !known || w <= 0 || h <= 0 || safe.w <= 0 || safe.h <= 0 {
            return Rect {
                x: 0.0,
                y: 0.0,
                w: pixels.0 as f32,
                h: pixels.1 as f32,
            };
        }
        let (sx, sy) = (pixels.0 as f32 / w as f32, pixels.1 as f32 / h as f32);
        Rect {
            x: safe.x as f32 * sx,
            y: safe.y as f32 * sy,
            w: safe.w as f32 * sx,
            h: safe.h as f32 * sy,
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: the renderer made for this window has been dropped first.
        unsafe { SDL_DestroyWindow(self.raw) };
    }
}

/// SDL's 2D renderer (Direct3D, Metal, OpenGL or Vulkan, as the system
/// offers) and the 160x144 texture it scales into the window.
pub struct Renderer {
    raw: *mut SDL_Renderer,
    texture: *mut SDL_Texture,
}

impl Renderer {
    /// A renderer for `window` that presents in step with the display.
    pub fn new(window: &Window, width: i32, height: i32) -> Result<Self, String> {
        // SAFETY: a live window; SDL picks the system's renderer.
        unsafe {
            let raw = SDL_CreateRenderer(window.raw, null());
            if raw.is_null() {
                return Err(format!("SDL_CreateRenderer: {}", error()));
            }
            SDL_SetRenderVSync(raw, 1);
            let texture = SDL_CreateTexture(
                raw,
                SDL_PIXELFORMAT_RGBA32,
                SDL_TEXTUREACCESS_STREAMING,
                width,
                height,
            );
            if texture.is_null() {
                let message = format!("SDL_CreateTexture: {}", error());
                SDL_DestroyRenderer(raw);
                return Err(message);
            }
            Ok(Self { raw, texture })
        }
    }

    /// Replaces the picture (RGBA8, the texture's size).
    pub fn upload(&mut self, rgba: &[u8], pitch: i32) {
        // SAFETY: a live texture; rgba holds height rows of pitch bytes.
        unsafe { SDL_UpdateTexture(self.texture, null(), rgba.as_ptr().cast(), pitch) };
    }

    pub fn set_scale_mode(&mut self, mode: SDL_ScaleMode) {
        // SAFETY: a live texture.
        unsafe { SDL_SetTextureScaleMode(self.texture, mode) };
    }

    /// The drawable size in pixels.
    pub fn output_size(&self) -> (u32, u32) {
        let (mut width, mut height) = (0, 0);
        // SAFETY: a live renderer, valid out pointers.
        unsafe { SDL_GetRenderOutputSize(self.raw, &mut width, &mut height) };
        (width.max(0) as u32, height.max(0) as u32)
    }

    /// Clears to black and draws the picture into `rect`.
    pub fn picture(&mut self, rect: (f32, f32, f32, f32)) {
        let dst = SDL_FRect {
            x: rect.0,
            y: rect.1,
            w: rect.2,
            h: rect.3,
        };
        // SAFETY: a live renderer and texture.
        unsafe {
            SDL_SetRenderDrawColor(self.raw, 0, 0, 0, 255);
            SDL_RenderClear(self.raw);
            if dst.w >= 1.0 && dst.h >= 1.0 {
                SDL_RenderTexture(self.raw, self.texture, null(), &dst);
            }
        }
    }

    /// Shows what was drawn; false if the renderer could not take a frame.
    pub fn present(&mut self) -> bool {
        // SAFETY: a live renderer.
        unsafe { SDL_RenderPresent(self.raw) }
    }

    /// Fills `rect` with white at `alpha` (0 to 1) over what is drawn.
    pub fn fill(&mut self, rect: Rect, alpha: f32) {
        let dst = SDL_FRect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
        };
        // SAFETY: a live renderer.
        unsafe {
            SDL_SetRenderDrawBlendMode(self.raw, SDL_BLENDMODE_BLEND);
            SDL_SetRenderDrawColorFloat(self.raw, 1.0, 1.0, 1.0, alpha);
            SDL_RenderFillRect(self.raw, &dst);
        }
    }

    /// Fills a disc of radius `r` around (x, y) with white at `alpha`.
    pub fn disc(&mut self, x: f32, y: f32, r: f32, alpha: f32) {
        const SEGMENTS: usize = 40;
        let color = SDL_FColor {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: alpha,
        };
        let vertex = |x, y| SDL_Vertex {
            position: SDL_FPoint { x, y },
            color,
            tex_coord: SDL_FPoint { x: 0.0, y: 0.0 },
        };
        let mut vertices = [vertex(x, y); SEGMENTS + 1];
        let mut indices = [0; SEGMENTS * 3];
        for i in 0..SEGMENTS {
            let angle = i as f32 * std::f32::consts::TAU / SEGMENTS as f32;
            vertices[i + 1] = vertex(x + r * angle.cos(), y + r * angle.sin());
            indices[i * 3 + 1] = i as i32 + 1;
            indices[i * 3 + 2] = ((i + 1) % SEGMENTS) as i32 + 1;
        }
        // SAFETY: a live renderer; the arrays outlive the call.
        unsafe {
            SDL_SetRenderDrawBlendMode(self.raw, SDL_BLENDMODE_BLEND);
            SDL_RenderGeometry(
                self.raw,
                null_mut(),
                vertices.as_ptr(),
                vertices.len() as i32,
                indices.as_ptr(),
                indices.len() as i32,
            );
        }
    }

    /// Writes `text` centred on (x, y) in SDL's built-in 8x8 font, scaled
    /// to `height` pixels, in white at `alpha`.
    pub fn label(&mut self, x: f32, y: f32, text: &str, height: f32, alpha: f32) {
        let scale = (height / 8.0).max(1.0);
        let width = 8.0 * text.chars().count() as f32;
        let text = CString::new(text).unwrap_or_default();
        // SAFETY: a live renderer; the text outlives the call.
        unsafe {
            SDL_SetRenderScale(self.raw, scale, scale);
            SDL_SetRenderDrawBlendMode(self.raw, SDL_BLENDMODE_BLEND);
            SDL_SetRenderDrawColorFloat(self.raw, 1.0, 1.0, 1.0, alpha);
            SDL_RenderDebugText(
                self.raw,
                x / scale - width / 2.0,
                y / scale - 4.0,
                text.as_ptr(),
            );
            SDL_SetRenderScale(self.raw, 1.0, 1.0);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: created in new and not used after this.
        unsafe {
            SDL_DestroyTexture(self.texture);
            SDL_DestroyRenderer(self.raw);
        }
    }
}
