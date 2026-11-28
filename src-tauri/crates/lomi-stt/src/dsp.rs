//! Conversion of captured audio to 16 kHz mono f32, plus level helpers.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

use crate::SAMPLE_RATE;

const BLOCK: usize = 1024;
const METER_WINDOW: usize = 800; // 50 ms at 16 kHz
const METER_FLOOR_DB: f32 = -60.0;
const SILENCE_AMPLITUDE: f32 = 1e-4; // -80 dBFS

pub fn i16_to_f32(x: i16) -> f32 {
    f32::from(x) / 32768.0
}

pub fn u16_to_f32(x: u16) -> f32 {
    (f32::from(x) - 32768.0) / 32768.0
}

/// Mono from interleaved frames; a trailing partial frame is ignored.
///
/// Stereo is averaged. Wider devices (audio interfaces expose 4–14 inputs,
/// usually with one microphone in use) give their loudest channel of this
/// chunk: averaging would cut that microphone by up to 23 dB.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        0 => Vec::new(),
        1 => interleaved.to_vec(),
        2 => interleaved
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[left, right]| (left + right) / 2.0)
            .collect(),
        n => {
            let frames = interleaved.chunks_exact(n);
            let energy = |c: usize| frames.clone().map(|f| f[c] * f[c]).sum::<f32>();
            let loudest = (0..n)
                .max_by(|&a, &b| energy(a).total_cmp(&energy(b)))
                .unwrap_or(0);
            frames.map(|f| f[loudest]).collect()
        }
    }
}

/// Streaming mono resampler to 16 kHz. Accepts arbitrary chunk sizes.
pub struct Resampler {
    /// `None` when the input is already 16 kHz.
    fft: Option<Fft<f32>>,
    rate: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
    /// Output delay samples still to be dropped from the start.
    skip: usize,
    input_total: usize,
    emitted: usize,
}

impl Resampler {
    pub fn new(input_rate: u32) -> Result<Self, String> {
        if input_rate == 0 {
            return Err("input sample rate must be positive".into());
        }
        let fft = if input_rate == SAMPLE_RATE {
            None
        } else {
            Some(
                Fft::new(
                    input_rate as usize,
                    SAMPLE_RATE as usize,
                    BLOCK,
                    1,
                    FixedSync::Input,
                )
                .map_err(|e| format!("cannot create resampler for {input_rate} Hz: {e}"))?,
            )
        };
        let skip = fft.as_ref().map_or(0, |f| f.output_delay());
        Ok(Self {
            fft,
            rate: input_rate as usize,
            pending: Vec::new(),
            out: Vec::new(),
            skip,
            input_total: 0,
            emitted: 0,
        })
    }

    pub fn push(&mut self, mono: &[f32]) -> Vec<f32> {
        let Some(fft) = self.fft.as_mut() else {
            return mono.to_vec();
        };
        self.input_total += mono.len();
        self.pending.extend_from_slice(mono);
        let mut result = Vec::new();
        let mut used = 0;
        while self.pending.len() - used >= fft.input_frames_next() {
            let n = fft.input_frames_next();
            run(
                fft,
                &self.pending[used..used + n],
                None,
                &mut self.out,
                &mut self.skip,
                &mut result,
            );
            used += n;
        }
        self.pending.drain(..used);
        self.emitted += result.len();
        result
    }

    /// Flushes the zero-padded remainder so the total output is
    /// `round(input_len * 16000 / rate)` samples.
    pub fn finish(&mut self) -> Vec<f32> {
        let Some(fft) = self.fft.as_mut() else {
            return Vec::new();
        };
        let target = (self.input_total as u64 * u64::from(SAMPLE_RATE) + self.rate as u64 / 2)
            / self.rate as u64;
        let target = target as usize;
        let mut result = Vec::new();
        let mut block = std::mem::take(&mut self.pending);
        while self.emitted + result.len() < target {
            let valid = block.len();
            block.resize(fft.input_frames_next(), 0.0);
            run(
                fft,
                &block,
                Some(valid),
                &mut self.out,
                &mut self.skip,
                &mut result,
            );
            block.clear();
        }
        result.truncate(target.saturating_sub(self.emitted));
        self.emitted += result.len();
        result
    }
}

fn run(
    fft: &mut Fft<f32>,
    input: &[f32],
    partial_len: Option<usize>,
    out: &mut Vec<f32>,
    skip: &mut usize,
    result: &mut Vec<f32>,
) {
    out.resize(fft.output_frames_max(), 0.0);
    let (Ok(inp), Ok(mut outp)) = (
        InterleavedSlice::new(input, 1, input.len()),
        InterleavedSlice::new_mut(out, 1, fft.output_frames_max()),
    ) else {
        return;
    };
    let idx = Indexing {
        input_offset: 0,
        output_offset: 0,
        partial_len,
        active_channels_mask: None,
    };
    // Sizes are fixed by the resampler's own queries, so an error is a bug;
    // dropping the block is the safe fallback in a processing thread.
    let Ok((_, produced)) = fft.process_into_buffer(&inp, &mut outp, Some(&idx)) else {
        return;
    };
    let drop = (*skip).min(produced);
    *skip -= drop;
    result.extend_from_slice(&out[drop..produced]);
}

/// 0..1 level of 50 ms windows for UI meters (-60 dBFS floor, sqrt curve).
#[derive(Default)]
pub struct LevelMeter {
    sum_sq: f32,
    count: usize,
}

impl LevelMeter {
    /// Returns the level of the latest window completed by this push.
    pub fn push(&mut self, pcm: &[f32]) -> Option<f32> {
        let mut latest = None;
        for &s in pcm {
            self.sum_sq += s * s;
            self.count += 1;
            if self.count == METER_WINDOW {
                let rms = (self.sum_sq / METER_WINDOW as f32).sqrt();
                let db = 20.0 * rms.max(1e-9).log10();
                latest = Some(
                    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB)
                        .clamp(0.0, 1.0)
                        .sqrt(),
                );
                self.sum_sq = 0.0;
                self.count = 0;
            }
        }
        latest
    }
}

/// True when every sample is below -80 dBFS (macOS denied-mic streams are zeros).
pub fn is_digital_silence(pcm: &[f32]) -> bool {
    pcm.iter().all(|s| s.abs() < SILENCE_AMPLITUDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn resample(rate: u32, input: &[f32], chunk: usize) -> Vec<f32> {
        let mut r = Resampler::new(rate).unwrap();
        let mut out = Vec::new();
        for c in input.chunks(chunk) {
            out.extend(r.push(c));
        }
        out.extend(r.finish());
        out
    }

    #[test]
    fn int_conversion() {
        assert_eq!(i16_to_f32(i16::MIN), -1.0);
        assert_eq!(i16_to_f32(0), 0.0);
        assert!(i16_to_f32(i16::MAX) < 1.0);
        assert_eq!(u16_to_f32(0), -1.0);
        assert_eq!(u16_to_f32(32768), 0.0);
        assert!(u16_to_f32(u16::MAX) < 1.0);
    }

    #[test]
    fn downmix_channels() {
        assert_eq!(downmix(&[0.1, 0.2], 1), vec![0.1, 0.2]);
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[6.0; 12], 6), vec![6.0, 6.0]);
        // Only the third of four inputs carries the microphone.
        let quad = [0.0, 0.01, 0.5, 0.0, 0.0, -0.01, -0.4, 0.0];
        assert_eq!(downmix(&quad, 4), vec![0.5, -0.4]);
        assert_eq!(downmix(&[1.0, 3.0, 9.0], 2), vec![2.0]);
        assert!(downmix(&[1.0], 0).is_empty());
    }

    #[test]
    fn rejects_zero_rate() {
        assert!(Resampler::new(0).is_err());
    }

    #[test]
    fn bypass_at_16k() {
        let input = sine(440.0, 16000, 1000);
        assert_eq!(resample(16000, &input, 333), input);
    }

    #[test]
    fn length_and_tone_levels() {
        for rate in [44100u32, 48000, 96000] {
            let n = rate as usize * 2;
            let expected = (n as f64 * 16000.0 / rate as f64).round() as i64;

            let out = resample(rate, &sine(1000.0, rate, n), 4800);
            assert!((out.len() as i64 - expected).abs() <= 1, "{rate}");
            let mid = &out[out.len() / 4..out.len() * 3 / 4];
            let db = 20.0 * (rms(mid) / rms(&sine(1000.0, 16000, 16000))).log10();
            assert!(db.abs() < 0.5, "{rate}: {db} dB");

            let out = resample(rate, &sine(10000.0, rate, n), 4800);
            assert!((out.len() as i64 - expected).abs() <= 1, "{rate}");
            let mid = &out[out.len() / 4..out.len() * 3 / 4];
            let db = 20.0 * (rms(mid) / rms(&sine(1000.0, 16000, 16000))).log10();
            assert!(db <= -40.0, "{rate}: {db} dB");
        }
    }

    #[test]
    fn chunking_does_not_change_output() {
        let input = sine(700.0, 48000, 20000);
        let whole = resample(48000, &input, input.len());
        let mut r = Resampler::new(48000).unwrap();
        let mut out = Vec::new();
        let mut rest = &input[..];
        for size in [1, 7, 480, 1000, 3333].iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (a, b) = rest.split_at((*size).min(rest.len()));
            out.extend(r.push(a));
            rest = b;
        }
        out.extend(r.finish());
        assert_eq!(out.len(), whole.len());
        assert!(out.iter().zip(&whole).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn level_meter() {
        let mut m = LevelMeter::default();
        assert_eq!(m.push(&[0.0; 800]), Some(0.0));
        assert_eq!(m.push(&[0.0; 799]), None);
        assert!(m.push(&[0.0]).is_some());
        let square: Vec<f32> = (0..800)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert!((m.push(&square).unwrap() - 1.0).abs() < 1e-6);
        assert!(m.push(&[0.001; 800]).unwrap() < 1e-2);
    }

    #[test]
    fn digital_silence() {
        assert!(is_digital_silence(&[]));
        assert!(is_digital_silence(&[0.0; 100]));
        assert!(is_digital_silence(&[3e-5, -3e-5]));
        assert!(!is_digital_silence(&[3.2e-4, 0.0]));
    }
}
