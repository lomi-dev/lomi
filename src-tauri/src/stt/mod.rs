//! Local dictation: one session at a time, audio only in memory.
//!
//! The webview never touches the microphone or the engine. A session thread
//! records through `capture`, streams 16 kHz audio to `lomi-stt-worker` and
//! reports progress through a `Channel`.

mod capture;
mod worker;

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lomi_stt::dsp::{is_digital_silence, LevelMeter};
use lomi_stt::protocol::{pcm_to_bytes, Request, Response};
use lomi_stt::SAMPLE_RATE;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{Manager, State, Window};

use crate::files::main_window;
use capture::{Capture, Chunk};
use worker::Worker;

const MODEL_FILE: &str = "parakeet-ultra-Q8_0.gguf";
const VAD_FILE: &str = "silero-vad-v6.2-F32.gguf";
const MAX_SAMPLES: usize = 5 * 60 * SAMPLE_RATE as usize;
/// Keeps the last syllable that is still in flight when Stop is pressed.
const STOP_TAIL: Duration = Duration::from_millis(250);
/// ~100 ms of audio per Feed frame.
const FEED_SAMPLES: usize = SAMPLE_RATE as usize / 10;
const RESULT_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct Stt(Arc<Mutex<Inner>>);

#[derive(Default)]
struct Inner {
    worker: Option<Worker>,
    active: Option<(u64, Sender<Control>)>,
    next_id: u64,
}

enum Control {
    Stop,
    Cancel,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Recording,
    Level {
        value: f32,
    },
    Transcribing,
    Result {
        text: String,
        #[serde(rename = "deviceLost")]
        device_lost: bool,
    },
    Error {
        code: &'static str,
        message: String,
    },
    Cancelled,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    available: bool,
    model_ready: bool,
}

fn models_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("stt")
        .join("models"))
}

#[tauri::command]
pub fn stt_status(window: Window, app: tauri::AppHandle) -> Result<Status, String> {
    main_window(&window)?;
    Ok(Status {
        available: worker::binary().is_ok(),
        model_ready: models_dir(&app)?.join(MODEL_FILE).is_file(),
    })
}

#[tauri::command]
pub fn stt_start(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Stt>,
    events: Channel<Event>,
) -> Result<u64, String> {
    main_window(&window)?;
    let dir = models_dir(&app)?;
    let model = dir.join(MODEL_FILE);
    if !model.is_file() {
        return Err("no-model".into());
    }
    let vad = Some(dir.join(VAD_FILE)).filter(|p| p.is_file());
    let mut inner = state.0.lock().map_err(|_| "busy")?;
    if inner.active.is_some() {
        return Err("busy".into());
    }
    inner.next_id += 1;
    let id = inner.next_id;
    let (control, controls) = mpsc::channel();
    inner.active = Some((id, control));
    drop(inner);
    let shared = state.0.clone();
    std::thread::Builder::new()
        .name("stt-session".into())
        .spawn(move || {
            let worker = shared.lock().ok().and_then(|mut i| i.worker.take());
            let worker = run_session(worker, model, vad, controls, &events);
            if let Ok(mut inner) = shared.lock() {
                inner.worker = worker;
                if inner
                    .active
                    .as_ref()
                    .is_some_and(|(active, _)| *active == id)
                {
                    inner.active = None;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn stt_stop(window: Window, state: State<'_, Stt>, id: u64) -> Result<(), String> {
    control(window, state, id, Control::Stop)
}

#[tauri::command]
pub fn stt_cancel(window: Window, state: State<'_, Stt>, id: u64) -> Result<(), String> {
    control(window, state, id, Control::Cancel)
}

fn control(window: Window, state: State<'_, Stt>, id: u64, message: Control) -> Result<(), String> {
    main_window(&window)?;
    let inner = state.0.lock().map_err(|_| "busy")?;
    match &inner.active {
        Some((active, sender)) if *active == id => {
            let _ = sender.send(message);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Runs one dictation session; returns the worker when it is still healthy.
fn run_session(
    worker: Option<Worker>,
    model: PathBuf,
    vad: Option<PathBuf>,
    controls: mpsc::Receiver<Control>,
    events: &Channel<Event>,
) -> Option<Worker> {
    let emit = |event: Event| {
        let _ = events.send(event);
    };
    let fail = |code: &'static str, message: String| {
        emit(Event::Error { code, message });
    };

    let (audio_tx, audio) = mpsc::channel();
    let capture = match Capture::start(move |chunk| {
        let _ = audio_tx.send(chunk);
    }) {
        Ok(capture) => capture,
        Err(message) => {
            fail("no-device", message);
            return worker;
        }
    };
    emit(Event::Recording);

    // The model loads while the user is already speaking; audio waits here.
    let (loaded_tx, loaded) = mpsc::channel();
    std::thread::spawn(move || {
        let result = worker
            .map_or_else(Worker::spawn, Ok)
            .and_then(|mut w| w.ensure_loaded(&model, vad.as_deref()).map(|()| w));
        let _ = loaded_tx.send(result);
    });

    let mut worker: Option<Worker> = None;
    let mut load_error = None;
    let mut pending: Vec<f32> = Vec::new();
    let mut total = 0usize;
    let mut heard = false;
    let mut device_lost = false;
    let mut meter = LevelMeter::default();
    let mut stop_at: Option<Instant> = None;
    let mut capture = Some(capture);

    loop {
        match controls.try_recv() {
            Ok(Control::Stop) if stop_at.is_none() => stop_at = Some(Instant::now() + STOP_TAIL),
            Ok(Control::Cancel) => {
                drop(capture.take());
                emit(Event::Cancelled);
                return worker.and_then(cancel);
            }
            _ => {}
        }
        if worker.is_none() && load_error.is_none() {
            match loaded.try_recv() {
                Ok(Ok(mut w)) => {
                    if begin(&mut w).is_ok() {
                        worker = Some(w);
                    } else {
                        load_error = Some("The speech engine stopped unexpectedly.".to_string());
                    }
                }
                Ok(Err(message)) => load_error = Some(message),
                Err(_) => {}
            }
        }
        if stop_at.is_some_and(|at| Instant::now() >= at) || total >= MAX_SAMPLES {
            if let Some(capture) = capture.take() {
                capture.finish();
            }
        }
        let finished = capture.is_none();
        let chunk = if finished {
            audio.try_recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            audio.recv_timeout(Duration::from_millis(20))
        };
        match chunk {
            Ok(Chunk::Audio(pcm)) => {
                heard |= !is_digital_silence(&pcm);
                if let Some(value) = meter.push(&pcm) {
                    emit(Event::Level { value });
                }
                total += pcm.len();
                pending.extend_from_slice(&pcm);
            }
            Ok(Chunk::Lost(message)) => {
                eprintln!("lomi: microphone lost: {message}");
                device_lost = true;
                stop_at = Some(Instant::now());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if finished => break,
            Err(RecvTimeoutError::Disconnected) => {}
        }
        if let Some(w) = worker.as_mut() {
            if pending.len() >= FEED_SAMPLES || (finished && !pending.is_empty()) {
                let frames = std::mem::take(&mut pending);
                if feed(w, &frames).is_err() {
                    worker = None;
                    load_error = Some("The speech engine stopped unexpectedly.".into());
                }
            }
        }
    }

    if !heard {
        fail(
            "silent-input",
            "The microphone delivers no sound. Check microphone access in system settings.".into(),
        );
        return worker.and_then(cancel);
    }
    emit(Event::Transcribing);
    let mut worker = match worker {
        Some(w) => w,
        None => match load_error.map_or_else(
            || {
                loaded
                    .recv()
                    .unwrap_or_else(|_| Err("The speech engine did not start.".into()))
            },
            Err,
        ) {
            Ok(mut w) => {
                if begin(&mut w).and_then(|()| feed(&mut w, &pending)).is_err() {
                    fail(
                        "engine-failed",
                        "The speech engine stopped unexpectedly.".into(),
                    );
                    return None;
                }
                w
            }
            Err(message) => {
                fail("engine-failed", message);
                return None;
            }
        },
    };
    if worker.send(&Request::Finish, &[]).is_err() {
        fail(
            "engine-failed",
            "The speech engine stopped unexpectedly.".into(),
        );
        return None;
    }
    match worker.recv(RESULT_TIMEOUT) {
        Ok(Response::Result { text, .. }) if text.is_empty() => {
            fail("no-speech", "Nothing was heard.".into());
            Some(worker)
        }
        Ok(Response::Result { text, .. }) => {
            emit(Event::Result { text, device_lost });
            Some(worker)
        }
        Ok(Response::Error { message, .. }) => {
            fail("engine-failed", message);
            Some(worker)
        }
        Ok(other) => {
            fail(
                "engine-failed",
                format!("Unexpected engine reply: {other:?}"),
            );
            None
        }
        Err(message) => {
            fail("engine-failed", message);
            None
        }
    }
}

fn begin(worker: &mut Worker) -> Result<(), String> {
    worker.send(
        &Request::Begin {
            language: None,
            vocabulary: Vec::new(),
            filter_silence: true,
        },
        &[],
    )
}

fn feed(worker: &mut Worker, pcm: &[f32]) -> Result<(), String> {
    // Frames stay far below the protocol payload cap.
    for frame in pcm.chunks(FEED_SAMPLES) {
        worker.send(&Request::Feed, &pcm_to_bytes(frame))?;
    }
    Ok(())
}

/// Cancels the worker's session; a worker that does not confirm is dropped (killed).
fn cancel(mut worker: Worker) -> Option<Worker> {
    worker.send(&Request::Cancel, &[]).ok()?;
    match worker.recv(CANCEL_TIMEOUT) {
        Ok(Response::Cancelled) => Some(worker),
        _ => None,
    }
}
