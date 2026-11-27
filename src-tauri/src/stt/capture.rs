//! Microphone capture: cpal callback -> lock-free ring -> processing thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use lomi_stt::dsp::{downmix, i16_to_f32, u16_to_f32, Resampler};

/// Two seconds of 48 kHz stereo; the processing thread drains every 10 ms.
const RING: usize = 48_000 * 2 * 2;

pub struct Capture {
    // Dropping the stream stops the device; it is not Send on every platform,
    // so it lives on (and dies with) the processing thread.
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub enum Chunk {
    /// 16 kHz mono audio.
    Audio(Vec<f32>),
    /// The device failed or disappeared; audio so far is still valid.
    Lost(String),
}

impl Capture {
    /// Opens the default input device in its native format and delivers
    /// 16 kHz mono chunks to `sink` until stopped.
    pub fn start(sink: impl FnMut(Chunk) + Send + 'static) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("stt-capture".into())
            .spawn(move || run(thread_stop, ready_tx, sink))
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("The microphone did not open.".into()),
        }
    }

    /// Stops the device and waits until every captured sample was delivered.
    pub fn finish(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn run(
    stop: Arc<AtomicBool>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
    mut sink: impl FnMut(Chunk),
) {
    let (stream, mut consumer, channels, rate, failed) = match open() {
        Ok(opened) => opened,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let mut resampler = match Resampler::new(rate) {
        Ok(r) => r,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    let mut interleaved = Vec::new();
    loop {
        let stopping = stop.load(Ordering::SeqCst);
        let available = consumer.slots();
        if let Ok(chunk) = consumer.read_chunk(available) {
            interleaved.extend(chunk);
        }
        let whole = interleaved.len() / channels * channels;
        if whole > 0 {
            let mono = downmix(&interleaved[..whole], channels);
            interleaved.drain(..whole);
            let out = resampler.push(&mono);
            if !out.is_empty() {
                sink(Chunk::Audio(out));
            }
        }
        if let Some(message) = failed.lock().ok().and_then(|mut m| m.take()) {
            drop(stream);
            flush(&mut resampler, &mut sink);
            sink(Chunk::Lost(message));
            return;
        }
        if stopping {
            drop(stream);
            flush(&mut resampler, &mut sink);
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn flush(resampler: &mut Resampler, sink: &mut impl FnMut(Chunk)) {
    let tail = resampler.finish();
    if !tail.is_empty() {
        sink(Chunk::Audio(tail));
    }
}

type Opened = (
    cpal::Stream,
    rtrb::Consumer<f32>,
    usize,
    u32,
    Arc<std::sync::Mutex<Option<String>>>,
);

fn open() -> Result<Opened, String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No microphone was found.")?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("The microphone cannot be opened: {e}"))?;
    let channels = usize::from(config.channels());
    let rate = config.sample_rate();
    let (mut producer, consumer) = rtrb::RingBuffer::<f32>::new(RING);
    let failed = Arc::new(std::sync::Mutex::new(None));
    let on_error = {
        let failed = failed.clone();
        move |e: cpal::Error| {
            if let Ok(mut slot) = failed.lock() {
                slot.get_or_insert_with(|| e.to_string());
            }
        }
    };
    // The callbacks run on the realtime audio thread: no allocation, no locks;
    // samples that do not fit the ring are dropped rather than blocking.
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config.config(),
            move |data: &[f32], _: &_| {
                for &s in data {
                    let _ = producer.push(s);
                }
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config.config(),
            move |data: &[i16], _: &_| {
                for &s in data {
                    let _ = producer.push(i16_to_f32(s));
                }
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            config.config(),
            move |data: &[u16], _: &_| {
                for &s in data {
                    let _ = producer.push(u16_to_f32(s));
                }
            },
            on_error,
            None,
        ),
        other => return Err(format!("Unsupported microphone format {other:?}.")),
    }
    .map_err(|e| format!("The microphone cannot be opened: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("The microphone cannot be started: {e}"))?;
    Ok((stream, consumer, channels, rate, failed))
}
