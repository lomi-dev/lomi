//! Local dictation: one session at a time, audio only in memory.
//!
//! The webview never touches the microphone or the engine. A session thread
//! records through `capture`, streams 16 kHz audio to `lomi-stt-worker` and
//! reports progress through a `Channel`.

mod capture;
pub mod models;
mod permission;
mod worker;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use lomi_stt::dsp::{is_digital_silence, LevelMeter};
use lomi_stt::protocol::{pcm_to_bytes, Acceleration, ErrorKind, Request, Response};
use lomi_stt::vocab::{self, Vocabulary};
use lomi_stt::SAMPLE_RATE;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{Manager, State, Window};

use crate::files::main_window;
use capture::{Capture, Chunk};
use worker::Worker;

const MAX_SAMPLES: usize = 5 * 60 * SAMPLE_RATE as usize;
/// Keeps the last syllable that is still in flight when Stop is pressed.
const STOP_TAIL: Duration = Duration::from_millis(250);
/// ~100 ms of audio per Feed frame.
const FEED_SAMPLES: usize = SAMPLE_RATE as usize / 10;
const RESULT_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);
/// A forgotten microphone must not keep recording the room.
/// Longest wait for a starting microphone to deliver sound before "Recording"
/// is shown anyway (a muted microphone stays digitally silent).
const MIC_WARMUP_MAX: Duration = Duration::from_millis(2500);
const SLEEP_GAP: Duration = Duration::from_secs(5);
const SILENCE_STOP: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Stt(Arc<Mutex<Inner>>);

#[derive(Default)]
struct Inner {
    worker: Option<Worker>,
    active: Option<(u64, Sender<Control>)>,
    next_id: u64,
    /// Set after the GPU failed once; kept until Lomi restarts.
    cpu_only: bool,
    /// Audio of the last failed recognition, so a retry needs no new speech.
    retry: Option<Retry>,
}

struct Retry {
    audio: Vec<f32>,
    model: PathBuf,
    vad: Option<PathBuf>,
    vocabulary: Arc<Vocabulary>,
    until: Instant,
}

/// Failed-recognition audio is dropped after this, never written anywhere.
const RETRY_KEEP: Duration = Duration::from_secs(120);

enum Failure {
    /// Engine crash, hang or GPU fault: the same audio may succeed on CPU.
    Retryable(String),
    /// The engine answered with an error; retrying the same input is pointless.
    Final(Option<Box<Worker>>, String),
}

impl Inner {
    fn acceleration(&self) -> Acceleration {
        if self.cpu_only {
            Acceleration::Cpu
        } else {
            Acceleration::Auto
        }
    }
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

/// The first downloaded model in catalog order.
fn ready_model(dir: &std::path::Path) -> Option<PathBuf> {
    lomi_stt::catalog::models()
        .iter()
        .find(|m| m.role == "asr" && models::is_ready(dir, m))
        .map(|m| models::path(dir, m))
}

impl Stt {
    /// Stops the idle engine so its model file can be removed.
    fn unload(&self) -> Result<(), String> {
        let mut inner = self.0.lock().map_err(|_| "busy")?;
        if inner.active.is_some() {
            return Err("busy".into());
        }
        inner.worker = None;
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    available: bool,
    model_ready: bool,
    permission: &'static str,
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
        model_ready: ready_model(&models_dir(&app)?).is_some(),
        permission: permission::state(),
    })
}

/// Opens the system's microphone privacy settings (fixed URLs only).
#[tauri::command]
pub fn stt_open_privacy_settings(window: Window) -> Result<(), String> {
    main_window(&window)?;
    let url = if cfg!(target_os = "macos") {
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
    } else if cfg!(windows) {
        "ms-settings:privacy-microphone"
    } else {
        return Err("unsupported".into());
    };
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// The loaded model holds hundreds of MB; free it after a quiet spell.
const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);

/// Drops the engine if no session started since session `id` ended.
fn unload_when_idle(shared: Arc<Mutex<Inner>>, id: u64) {
    let _ = std::thread::Builder::new()
        .name("stt-idle".into())
        .spawn(move || {
            std::thread::sleep(IDLE_UNLOAD);
            let idle = shared
                .lock()
                .ok()
                .filter(|inner| inner.active.is_none() && inner.next_id == id)
                .and_then(|mut inner| inner.worker.take());
            drop(idle);
        });
}

/// Per-device dictation preferences from Settings → Speech.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    microphones: Option<Vec<String>>,
    gain_db: Option<f32>,
    vocabulary: Vec<String>,
}

#[tauri::command]
pub fn stt_start(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, Stt>,
    events: Channel<Event>,
    model_id: Option<String>,
    preferences: Option<Preferences>,
) -> Result<u64, String> {
    main_window(&window)?;
    let preferences = preferences.unwrap_or_default();
    let microphones = microphone_list(preferences.microphones)?;
    let gain_db = capture::gain_db(preferences.gain_db)?;
    if preferences.vocabulary.len() > 4 * vocab::MAX_ENTRIES {
        return Err("The vocabulary has too many entries.".into());
    }
    let vocabulary = Arc::new(Vocabulary::parse(&preferences.vocabulary)?);
    let dir = models_dir(&app)?;
    // Only catalog ids are accepted; a missing choice falls back to any ready model.
    let chosen = model_id
        .and_then(|id| lomi_stt::catalog::find(&id))
        .filter(|m| m.role == "asr" && models::is_ready(&dir, m))
        .map(|m| models::path(&dir, m));
    let Some(model) = chosen.or_else(|| ready_model(&dir)) else {
        eprintln!("lomi: dictation failed: no-model in {}", dir.display());
        return Err("no-model".into());
    };
    let vad_model = lomi_stt::catalog::vad();
    let vad = models::is_ready(&dir, vad_model).then(|| models::path(&dir, vad_model));
    let mut inner = state.0.lock().map_err(|_| "busy")?;
    if inner.active.is_some() {
        return Err("busy".into());
    }
    inner.next_id += 1;
    let id = inner.next_id;
    let (control, controls) = mpsc::channel();
    inner.active = Some((id, control));
    let acceleration = inner.acceleration();
    inner.retry = None;
    drop(inner);
    let shared = state.0.clone();
    std::thread::Builder::new()
        .name("stt-session".into())
        .spawn(move || {
            let worker = shared.lock().ok().and_then(|mut i| i.worker.take());
            let worker = run_session(
                worker,
                Setup {
                    model,
                    vad,
                    microphones,
                    gain_db,
                    acceleration,
                    vocabulary,
                },
                controls,
                &events,
                &shared,
            );
            if let Ok(mut inner) = shared.lock() {
                inner.worker = worker;
                unload_when_idle(shared.clone(), id);
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

/// Starts the engine process ahead of the first dictation: its first launch
/// compiles GPU shaders, which can take tens of seconds.
#[tauri::command]
pub fn stt_prepare(window: Window, state: State<'_, Stt>) -> Result<(), String> {
    main_window(&window)?;
    {
        let inner = state.0.lock().map_err(|_| "busy")?;
        if inner.worker.is_some() || inner.active.is_some() || worker::binary().is_err() {
            return Ok(());
        }
    }
    let shared = state.0.clone();
    std::thread::spawn(move || {
        let Ok(spawned) = Worker::spawn() else { return };
        if let Ok(mut inner) = shared.lock() {
            // A session that started meanwhile spawned its own worker.
            if inner.worker.is_none() && inner.active.is_none() {
                inner.worker = Some(spawned);
            }
        }
    });
    Ok(())
}

fn microphone_list(microphones: Option<Vec<String>>) -> Result<Vec<String>, String> {
    let microphones = microphones.unwrap_or_default();
    if microphones.len() > 64 || microphones.iter().any(|id| id.len() > 1024) {
        return Err("Invalid microphone list.".into());
    }
    Ok(microphones)
}

/// The Settings microphone test: live levels of the chosen input, nothing
/// recorded or recognized. The gain can change while it runs.
#[derive(Default)]
pub struct MicTest {
    capture: Mutex<Option<Capture>>,
    gain_bits: Arc<AtomicU32>,
}

#[tauri::command]
pub async fn stt_mic_test_start(
    window: Window,
    state: State<'_, MicTest>,
    microphones: Option<Vec<String>>,
    gain_db: Option<f32>,
    levels: Channel<f32>,
) -> Result<(), String> {
    main_window(&window)?;
    let microphones = microphone_list(microphones)?;
    state
        .gain_bits
        .store(capture::gain_db(gain_db)?.to_bits(), Ordering::Relaxed);
    let gain_bits = state.gain_bits.clone();
    let mut meter = LevelMeter::default();
    // Opening a device can block for seconds; keep it off the IPC thread.
    let capture = tauri::async_runtime::spawn_blocking(move || {
        Capture::start(microphones, move |chunk| match chunk {
            Chunk::Audio(mut pcm) => {
                capture::apply_gain(&mut pcm, f32::from_bits(gain_bits.load(Ordering::Relaxed)));
                if let Some(level) = meter.push(&pcm) {
                    let _ = levels.send(level);
                }
            }
            // A negative level tells the page the device stopped.
            Chunk::Lost(message) => {
                eprintln!("lomi: microphone test stopped: {message}");
                let _ = levels.send(-1.0);
            }
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    // Replacing a running test stops it.
    *state.capture.lock().map_err(|_| "busy")? = Some(capture);
    Ok(())
}

#[tauri::command]
pub fn stt_mic_test_gain(
    window: Window,
    state: State<'_, MicTest>,
    gain_db: f32,
) -> Result<(), String> {
    main_window(&window)?;
    let db = capture::gain_db(Some(gain_db))?;
    state.gain_bits.store(db.to_bits(), Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub fn stt_mic_test_stop(window: Window, state: State<'_, MicTest>) -> Result<(), String> {
    main_window(&window)?;
    drop(state.capture.lock().map_err(|_| "busy")?.take());
    Ok(())
}

/// Connected microphones for the Speech settings priority list.
#[tauri::command]
pub async fn stt_list_microphones(window: Window) -> Result<Vec<capture::Microphone>, String> {
    main_window(&window)?;
    // Device enumeration can block on CoreAudio/ALSA; keep it off the IPC thread.
    tauri::async_runtime::spawn_blocking(capture::microphones)
        .await
        .map_err(|e| e.to_string())
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
/// What a session records with and recognizes with.
struct Setup {
    model: PathBuf,
    vad: Option<PathBuf>,
    microphones: Vec<String>,
    gain_db: f32,
    acceleration: Acceleration,
    vocabulary: Arc<Vocabulary>,
}

fn run_session(
    worker: Option<Worker>,
    setup: Setup,
    controls: mpsc::Receiver<Control>,
    events: &Channel<Event>,
    shared: &Mutex<Inner>,
) -> Option<Worker> {
    let Setup {
        model,
        vad,
        microphones,
        gain_db,
        acceleration,
        vocabulary,
    } = setup;
    let terms = vocabulary.terms();
    let emit = |event: Event| {
        let _ = events.send(event);
    };
    let fail = |code: &'static str, message: String| {
        // Codes and engine messages only; transcripts are never logged.
        eprintln!("lomi: dictation failed: {code}: {message}");
        emit(Event::Error { code, message });
    };

    if permission::ensure() == permission::Access::Denied {
        fail(
            "permission-denied",
            "Lomi has no access to the microphone.".into(),
        );
        return worker;
    }
    let (audio_tx, audio) = mpsc::channel();
    let capture = match Capture::start(microphones, move |mut chunk| {
        if let Chunk::Audio(pcm) = &mut chunk {
            capture::apply_gain(pcm, gain_db);
        }
        let _ = audio_tx.send(chunk);
    }) {
        Ok(capture) => capture,
        Err(message) => {
            fail("no-device", message);
            return worker;
        }
    };
    // Some USB interfaces (a Focusrite Vocaster on Windows) deliver ~1.5 s of
    // exact zeros after the device starts. "Recording" is announced with the
    // first real sound, so the user starts speaking when the microphone hears.
    let mut announced = false;
    let warmup_until = Instant::now() + MIC_WARMUP_MAX;

    // The model loads while the user is already speaking; audio waits here.
    let (loaded_tx, loaded) = mpsc::channel();
    let (load_model, load_vad) = (model.clone(), vad.clone());
    std::thread::spawn(move || {
        let result = worker.map_or_else(Worker::spawn, Ok).and_then(|mut w| {
            w.ensure_loaded(&load_model, load_vad.as_deref(), acceleration)
                .map(|()| w)
        });
        let _ = loaded_tx.send(result);
    });

    let mut worker: Option<Worker> = None;
    let mut load_error = None;
    let mut pending: Vec<f32> = Vec::new();
    // Everything captured, for a replay on a fresh worker (bounded by MAX_SAMPLES).
    let mut recorded: Vec<f32> = Vec::new();
    let mut total = 0usize;
    let mut heard = false;
    let mut device_lost = false;
    let mut meter = LevelMeter::default();
    let mut stop_at: Option<Instant> = None;
    // Without a VAD there are no speech edges, so only the 5 min limit applies.
    let mut quiet_since = Some(Instant::now());
    let mut capture = Some(capture);
    // macOS and Linux monotonic clocks pause during system sleep while the
    // wall clock runs on; a gap between them ends the session. A slow engine
    // start blocks both clocks alike, so it never looks like sleep.
    // shortcut: Windows' monotonic clock counts sleep, so there the device
    // error ends the session instead; listen for power events if that fails.
    let mut clocks = (Instant::now(), SystemTime::now());

    loop {
        let now = (Instant::now(), SystemTime::now());
        let mono = now.0 - clocks.0;
        let wall = now.1.duration_since(clocks.1).unwrap_or_default();
        clocks = now;
        if stop_at.is_none() && wall > mono + SLEEP_GAP {
            eprintln!("lomi: dictation stopped after system sleep");
            stop_at = Some(Instant::now());
        }
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
                    if begin(&mut w, &terms).is_ok() {
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
                if !announced && (heard || Instant::now() >= warmup_until) {
                    announced = true;
                    emit(Event::Recording);
                }
                if let Some(value) = meter.push(&pcm) {
                    emit(Event::Level { value });
                }
                total += pcm.len();
                pending.extend_from_slice(&pcm);
                recorded.extend_from_slice(&pcm);
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
            for speaking in w.speech_edges() {
                quiet_since = (!speaking).then(Instant::now);
            }
            if stop_at.is_none() && quiet_since.is_some_and(|q| q.elapsed() >= SILENCE_STOP) {
                stop_at = Some(Instant::now());
            }
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
    let first = match worker {
        // Fed while recording; only the finish is left.
        Some(w) => Ok(w),
        None => load_error
            .map_or_else(
                || {
                    loaded
                        .recv()
                        .unwrap_or_else(|_| Err("The speech engine did not start.".into()))
                },
                Err,
            )
            .and_then(|mut w| {
                begin(&mut w, &terms)?;
                feed(&mut w, &recorded)?;
                Ok(w)
            })
            .map_err(Failure::Retryable),
    };
    let outcome = match first.and_then(finish) {
        Err(Failure::Retryable(message)) if acceleration != Acceleration::Cpu => {
            eprintln!("lomi: speech engine failed, retrying on CPU: {message}");
            if let Ok(mut inner) = shared.lock() {
                inner.cpu_only = true;
            }
            replay(&model, vad.as_deref(), &recorded, &terms, Acceleration::Cpu)
        }
        other => other,
    };
    report(outcome, device_lost, events, shared, &vocabulary, || {
        Retry {
            audio: recorded,
            model,
            vad,
            vocabulary: vocabulary.clone(),
            until: Instant::now() + RETRY_KEEP,
        }
    })
}

/// Sends the recognition outcome to the webview; keeps audio for a manual retry.
fn report(
    outcome: Result<(Worker, String), Failure>,
    device_lost: bool,
    events: &Channel<Event>,
    shared: &Mutex<Inner>,
    vocabulary: &Vocabulary,
    retry: impl FnOnce() -> Retry,
) -> Option<Worker> {
    let outcome = outcome.map(|(worker, text)| (worker, vocabulary.correct(&text)));
    let fail = |code: &'static str, message: String| {
        eprintln!("lomi: dictation failed: {code}: {message}");
        let _ = events.send(Event::Error { code, message });
    };
    match outcome {
        Ok((worker, text)) if text.is_empty() => {
            fail("no-speech", "Nothing was heard.".into());
            Some(worker)
        }
        Ok((worker, text)) => {
            let _ = events.send(Event::Result { text, device_lost });
            Some(worker)
        }
        Err(Failure::Final(worker, message)) => {
            fail("engine-failed", message);
            worker.map(|w| *w)
        }
        Err(Failure::Retryable(message)) => {
            if let Ok(mut inner) = shared.lock() {
                inner.retry = Some(retry());
            }
            fail("engine-retry", message);
            None
        }
    }
}

/// Recognizes `audio` on a fresh worker.
fn replay(
    model: &std::path::Path,
    vad: Option<&std::path::Path>,
    audio: &[f32],
    terms: &[String],
    acceleration: Acceleration,
) -> Result<(Worker, String), Failure> {
    let mut worker = Worker::spawn().map_err(Failure::Retryable)?;
    worker
        .ensure_loaded(model, vad, acceleration)
        .and_then(|()| begin(&mut worker, terms))
        .and_then(|()| feed(&mut worker, audio))
        .map_err(Failure::Retryable)?;
    finish(worker)
}

fn finish(mut worker: Worker) -> Result<(Worker, String), Failure> {
    worker
        .send(&Request::Finish, &[])
        .map_err(Failure::Retryable)?;
    match worker.recv(RESULT_TIMEOUT) {
        Ok(Response::Result { text, .. }) => Ok((worker, text)),
        Ok(Response::Error {
            kind: ErrorKind::GpuFault,
            message,
        }) => Err(Failure::Retryable(message)),
        Ok(Response::Error { message, .. }) => Err(Failure::Final(Some(Box::new(worker)), message)),
        Ok(other) => Err(Failure::Final(
            None,
            format!("Unexpected engine reply: {other:?}"),
        )),
        Err(message) => Err(Failure::Retryable(message)),
    }
}

/// Recognizes the audio kept from the last failed attempt.
#[tauri::command]
pub fn stt_retry(
    window: Window,
    state: State<'_, Stt>,
    events: Channel<Event>,
) -> Result<u64, String> {
    main_window(&window)?;
    let mut inner = state.0.lock().map_err(|_| "busy")?;
    if inner.active.is_some() {
        return Err("busy".into());
    }
    let retry = inner
        .retry
        .take()
        .filter(|r| r.until > Instant::now())
        .ok_or("expired")?;
    let acceleration = inner.acceleration();
    inner.next_id += 1;
    let id = inner.next_id;
    // A retry cannot be stopped or cancelled; the sender only marks activity.
    inner.active = Some((id, mpsc::channel().0));
    drop(inner);
    let shared = state.0.clone();
    std::thread::Builder::new()
        .name("stt-retry".into())
        .spawn(move || {
            let _ = events.send(Event::Transcribing);
            let vocabulary = retry.vocabulary.clone();
            let outcome = replay(
                &retry.model,
                retry.vad.as_deref(),
                &retry.audio,
                &vocabulary.terms(),
                acceleration,
            );
            let worker = report(outcome, false, &events, &shared, &vocabulary, || Retry {
                until: Instant::now() + RETRY_KEEP,
                ..retry
            });
            if let Ok(mut inner) = shared.lock() {
                inner.worker = worker;
                unload_when_idle(shared.clone(), id);
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

fn begin(worker: &mut Worker, terms: &[String]) -> Result<(), String> {
    worker.send(
        &Request::Begin {
            language: None,
            vocabulary: terms.to_vec(),
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
