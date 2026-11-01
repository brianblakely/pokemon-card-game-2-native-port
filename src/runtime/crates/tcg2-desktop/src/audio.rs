//! The game's sound through an SDL3 audio stream.
//!
//! The kernel renders exactly the console's sample count per frame against
//! its own clock; the host paces frames by the system clock, and the sound
//! card plays by its own. A slight resampling ratio (at most 0.5%, inaudible)
//! keeps the queue near its target instead of letting it drift into gaps or
//! latency. While the game's sound unit is fully off the device is paused,
//! so an idle sound card can power down.

use sdl3_sys::everything::*;

/// Queue target and pre-roll, in game frames of sound.
const TARGET_FRAMES: f32 = 3.0;
/// Beyond this much queued sound, drop it (the host fell behind and caught up).
const MAX_FRAMES: f32 = 12.0;
/// How long the sound unit must stay off before the device is paused.
const SILENT_FRAMES_TO_PAUSE: u32 = 120;

pub struct Audio {
    stream: *mut SDL_AudioStream,
    samples_per_frame: f32,
    started: bool,
    silent_frames: u32,
    idle: bool,
    pub volume: f32,
    scratch: Vec<f32>,
}

impl Audio {
    /// Opens the default playback device; None if there is none (the game
    /// then runs silent).
    pub fn open(rate: u32, frame_rate: f64) -> Option<Self> {
        let spec = SDL_AudioSpec {
            format: SDL_AUDIO_F32,
            channels: 2,
            freq: rate as i32,
        };
        // SAFETY: SDL is initialised with audio; a null callback queues data.
        let stream = unsafe {
            SDL_OpenAudioDeviceStream(
                SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                &spec,
                None,
                std::ptr::null_mut(),
            )
        };
        if stream.is_null() {
            crate::warn(&format!("no sound: {}", crate::sdl::error()));
            return None;
        }
        Some(Self {
            stream,
            samples_per_frame: (rate as f64 / frame_rate) as f32,
            started: false,
            silent_frames: 0,
            idle: false,
            volume: 1.0,
            scratch: Vec::new(),
        })
    }

    fn queued_frames(&self) -> f32 {
        // SAFETY: an open stream.
        let bytes = unsafe { SDL_GetAudioStreamQueued(self.stream) }.max(0) as f32;
        bytes / 8.0 / self.samples_per_frame
    }

    /// Queues one frame's sound (interleaved stereo). `silent`: the sound
    /// unit has every channel's DAC off.
    pub fn queue(&mut self, samples: &[f32], silent: bool) {
        if silent {
            self.silent_frames = self.silent_frames.saturating_add(1);
            if self.silent_frames >= SILENT_FRAMES_TO_PAUSE && !self.idle {
                self.idle = true;
                self.stop();
            }
            if self.idle {
                return;
            }
        } else {
            self.silent_frames = 0;
            self.idle = false;
        }
        if samples.is_empty() {
            return;
        }
        self.scratch.clear();
        self.scratch.extend(samples.iter().map(|s| s * self.volume));
        // SAFETY: an open stream; SDL copies the bytes.
        unsafe {
            if self.queued_frames() > MAX_FRAMES {
                SDL_ClearAudioStream(self.stream);
            }
            SDL_PutAudioStreamData(
                self.stream,
                self.scratch.as_ptr().cast(),
                (self.scratch.len() * 4) as i32,
            );
            let queued = self.queued_frames();
            if !self.started && queued >= TARGET_FRAMES {
                self.started = SDL_ResumeAudioStreamDevice(self.stream);
            }
            if self.started {
                let error = (queued - TARGET_FRAMES) / TARGET_FRAMES;
                let ratio = 1.0 + (error * 0.002).clamp(-0.005, 0.005);
                SDL_SetAudioStreamFrequencyRatio(self.stream, ratio);
            }
        }
    }

    /// Pauses the device and drops what is queued; the next sound pre-rolls
    /// again.
    pub fn stop(&mut self) {
        self.started = false;
        // SAFETY: an open stream.
        unsafe {
            SDL_PauseAudioStreamDevice(self.stream);
            SDL_ClearAudioStream(self.stream);
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        // SAFETY: opened by SDL_OpenAudioDeviceStream and destroyed once.
        unsafe { SDL_DestroyAudioStream(self.stream) };
    }
}
