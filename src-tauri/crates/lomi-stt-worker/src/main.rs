//! `lomi-stt-worker`: runs the transcription engine for one Lomi process.
//!
//! Speaks `lomi_stt::protocol` on stdin/stdout. Logs go to stderr and never
//! contain transcript text or audio. The process exits as soon as stdin closes,
//! even while native inference is running, so it cannot outlive Lomi.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::sync::mpsc;
use std::thread;

use lomi_stt::protocol::{
    bytes_to_pcm, read_frame, write_frame, Acceleration, DeviceInfo, ErrorKind, Request, Response,
    PROTOCOL_VERSION,
};
use lomi_stt::SAMPLE_RATE;
use transcribe_cpp::{
    Backend, CancelToken, Feature, Model, ModelOptions, RunOptions, Session, VadOptions,
};

/// Five minutes of audio plus margin for the stop tail.
const MAX_SESSION_SAMPLES: usize = (5 * 60 + 5) * SAMPLE_RATE as usize;

fn main() {
    // ggml prints to stdout in places; any stray byte would corrupt the framing,
    // so the protocol takes the real stdout and fd 1 is pointed at stderr before
    // native code runs.
    let out = match take_stdout() {
        Ok(file) => file,
        Err(e) => {
            eprintln!("lomi-stt-worker: cannot claim stdout: {e}");
            std::process::exit(3);
        }
    };
    let mut out = BufWriter::new(out);

    let cancel = CancelToken::new();
    let (requests, inbox) = mpsc::channel();
    let reader_cancel = cancel.clone();
    thread::spawn(move || read_requests(requests, reader_cancel));

    let devices = transcribe_cpp::devices()
        .into_iter()
        .map(|d| DeviceInfo {
            kind: d.kind,
            name: d.name,
            description: d.description,
        })
        .collect();
    send(
        &mut out,
        &Response::Hello {
            protocol: PROTOCOL_VERSION,
            engine: transcribe_cpp::version().to_string(),
            devices,
        },
    );

    let mut engine = Engine::default();
    for (request, payload) in inbox {
        if let Some(response) = engine.handle(request, &payload, &cancel) {
            send(&mut out, &response);
        }
    }
}

fn send(out: &mut impl Write, response: &Response) {
    if write_frame(out, response, &[]).is_err() {
        // The parent is gone; nothing left to serve.
        exit_now(0);
    }
}

/// Reads frames on a dedicated thread so a hung native run never stops EOF
/// detection, and so `Cancel` can abort a run in flight.
fn read_requests(requests: mpsc::Sender<(Request, Vec<u8>)>, cancel: CancelToken) {
    let mut input = BufReader::new(io::stdin().lock());
    loop {
        match read_frame::<Request>(&mut input) {
            Ok(Some((request, payload))) => {
                if request == Request::Cancel {
                    cancel.cancel();
                }
                if requests.send((request, payload)).is_err() {
                    exit_now(0);
                }
            }
            Ok(None) => exit_now(0),
            Err(e) => {
                eprintln!("lomi-stt-worker: protocol error: {e}");
                exit_now(2);
            }
        }
    }
}

/// Skips destructors and atexit handlers: native teardown can block while a
/// run is still in flight on the main thread.
fn exit_now(code: i32) -> ! {
    unsafe { libc::_exit(code) }
}

#[derive(Default)]
struct Engine {
    asr: Option<(Model, Session)>,
    vad: Option<Model>,
    options: RunOptions,
    filter_silence: bool,
    audio: Vec<f32>,
    active: bool,
}

impl Engine {
    fn handle(
        &mut self,
        request: Request,
        payload: &[u8],
        cancel: &CancelToken,
    ) -> Option<Response> {
        match request {
            Request::Load {
                model,
                vad,
                acceleration,
            } => Some(self.load(&model, vad.as_deref(), acceleration)),
            Request::Begin {
                language,
                vocabulary,
                filter_silence,
            } => {
                let Some((model, _)) = &self.asr else {
                    return Some(error(ErrorKind::Input, "no model loaded"));
                };
                let vocabulary_supported = model.supports(Feature::Vocabulary);
                self.options = RunOptions {
                    language,
                    vocabulary: if vocabulary_supported {
                        vocabulary
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                };
                self.filter_silence = filter_silence;
                self.audio.clear();
                self.active = true;
                cancel.reset();
                None
            }
            Request::Feed => {
                if !self.active {
                    return None;
                }
                match bytes_to_pcm(payload) {
                    Ok(pcm) if self.audio.len() + pcm.len() <= MAX_SESSION_SAMPLES => {
                        self.audio.extend_from_slice(&pcm);
                        None
                    }
                    Ok(_) => Some(error(ErrorKind::Input, "session audio limit exceeded")),
                    Err(e) => Some(error(ErrorKind::Input, &e.to_string())),
                }
            }
            Request::Finish => {
                if !self.active {
                    return Some(error(ErrorKind::Input, "no active session"));
                }
                self.active = false;
                Some(self.transcribe(cancel))
            }
            Request::Cancel => {
                self.active = false;
                self.audio = Vec::new();
                cancel.reset();
                Some(Response::Cancelled)
            }
        }
    }

    fn load(&mut self, path: &str, vad: Option<&str>, acceleration: Acceleration) -> Response {
        if let Err(message) = check_cpu() {
            return error(ErrorKind::UnsupportedCpu, &message);
        }
        self.asr = None;
        let options = ModelOptions {
            backend: match acceleration {
                Acceleration::Auto => Backend::Auto,
                Acceleration::Cpu => Backend::Cpu,
            },
            ..Default::default()
        };
        let model = match Model::load_with(path, &options) {
            Ok(model) => model,
            Err(e) => return engine_error(&e),
        };
        self.vad = match vad.map(Model::load) {
            Some(Ok(vad)) => Some(vad),
            Some(Err(e)) => {
                // Dictation without silence filtering still works.
                eprintln!("lomi-stt-worker: VAD unavailable: {e}");
                None
            }
            None => None,
        };
        let session = match model.session() {
            Ok(session) => session,
            Err(e) => return engine_error(&e),
        };
        let languages = model
            .capabilities()
            .map(|c| c.languages)
            .unwrap_or_default();
        let response = Response::Loaded {
            backend: model.backend(),
            languages,
            vocabulary: model.supports(Feature::Vocabulary),
            long_form: model.supports(Feature::LongForm),
        };
        self.asr = Some((model, session));
        response
    }

    fn transcribe(&mut self, cancel: &CancelToken) -> Response {
        let audio = std::mem::take(&mut self.audio);
        let audio_ms = audio.len() as u64 * 1000 / u64::from(SAMPLE_RATE);
        let speech_ms = match (&self.vad, self.filter_silence) {
            (Some(vad), true) => match speech_ms(vad, &audio) {
                Ok(ms) => Some(ms),
                Err(e) => {
                    // Never lose a recording because of the VAD.
                    eprintln!("lomi-stt-worker: VAD failed, transcribing raw audio: {e}");
                    None
                }
            },
            _ => None,
        };
        if speech_ms == Some(0) {
            return Response::Result {
                text: String::new(),
                audio_ms,
                speech_ms: 0,
            };
        }
        let Some((_, session)) = self.asr.as_mut() else {
            return error(ErrorKind::Input, "no model loaded");
        };
        session.set_cancel_token(cancel);
        let result = session.run(&audio, &self.options);
        session.clear_cancel_token();
        match result {
            Ok(transcript) => Response::Result {
                text: transcript.text.trim().to_string(),
                audio_ms,
                speech_ms: speech_ms.unwrap_or(audio_ms),
            },
            Err(e) if e.kind() == transcribe_cpp::ErrorKind::Aborted => Response::Cancelled,
            Err(e) => engine_error(&e),
        }
    }
}

fn speech_ms(vad: &Model, audio: &[f32]) -> Result<u64, transcribe_cpp::Error> {
    let options = VadOptions {
        threshold: 0.3,
        ..Default::default()
    };
    let result = vad.vad_session()?.run(audio, &options)?;
    let samples: i64 = result
        .segments
        .iter()
        .map(|s| s.end_sample - s.start_sample)
        .sum();
    Ok(samples.max(0) as u64 * 1000 / u64::from(SAMPLE_RATE))
}

fn engine_error(e: &transcribe_cpp::Error) -> Response {
    use transcribe_cpp::ErrorKind as E;
    let kind = match e.kind() {
        E::Backend => ErrorKind::GpuFault,
        E::OutOfMemory => ErrorKind::OutOfMemory,
        E::ModelFileNotFound | E::ModelLoad | E::VersionMismatch | E::Unsupported => {
            ErrorKind::Model
        }
        E::InvalidArgument | E::InputTooLong | E::InputTooShort => ErrorKind::Input,
        _ => ErrorKind::Internal,
    };
    error(kind, &e.to_string())
}

fn error(kind: ErrorKind, message: &str) -> Response {
    Response::Error {
        kind,
        message: message.to_string(),
    }
}

/// Windows/Linux x64 build with GGML_NATIVE=OFF, whose default ISA floor is
/// SSE4.2+AVX+AVX2+BMI2+FMA+F16C; refuse cleanly instead of crashing with
/// SIGILL. Intel macOS is excluded: transcribe-cpp-sys pins a lower floor there.
#[cfg(all(target_arch = "x86_64", not(target_os = "macos")))]
fn check_cpu() -> Result<(), String> {
    let missing: Vec<&str> = [
        ("sse4.2", std::arch::is_x86_feature_detected!("sse4.2")),
        ("avx", std::arch::is_x86_feature_detected!("avx")),
        ("avx2", std::arch::is_x86_feature_detected!("avx2")),
        ("bmi2", std::arch::is_x86_feature_detected!("bmi2")),
        ("fma", std::arch::is_x86_feature_detected!("fma")),
        ("f16c", std::arch::is_x86_feature_detected!("f16c")),
    ]
    .into_iter()
    .filter_map(|(name, present)| (!present).then_some(name))
    .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("this CPU lacks {}", missing.join(", ")))
    }
}

#[cfg(not(all(target_arch = "x86_64", not(target_os = "macos"))))]
fn check_cpu() -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn take_stdout() -> io::Result<File> {
    use std::os::fd::FromRawFd;
    unsafe {
        let fd = libc::dup(libc::STDOUT_FILENO);
        if fd < 0 || libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_fd(fd))
    }
}

#[cfg(windows)]
fn take_stdout() -> io::Result<File> {
    use std::os::windows::io::FromRawHandle;
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 || libc::dup2(2, 1) < 0 {
            return Err(io::Error::last_os_error());
        }
        let handle = libc::get_osfhandle(fd);
        if handle == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_handle(handle as _))
    }
}
