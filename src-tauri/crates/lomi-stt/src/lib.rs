//! Dictation core shared by Lomi and `lomi-stt-worker`.
//!
//! Audio is 16 kHz mono f32 in [-1, 1] everywhere past `dsp`.

pub mod catalog;
pub mod dsp;
pub mod protocol;
pub mod vad;

/// Sample rate of every buffer that crosses the worker boundary.
pub const SAMPLE_RATE: u32 = 16_000;
