//! Drives the real worker binary over its protocol.
//!
//! Needs model files, so it runs only when `LOMI_STT_TEST_MODEL` (ASR GGUF)
//! is set; `LOMI_STT_TEST_VAD` (Silero GGUF) and `LOMI_STT_TEST_WAV`
//! (16 kHz mono speech) enable the silence and transcript checks.

use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use lomi_stt::protocol::{
    pcm_to_bytes, read_frame, write_frame, Acceleration, Request, Response, PROTOCOL_VERSION,
};

struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lomi-stt-worker"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut worker = Self {
            child,
            stdin,
            stdout,
        };
        match worker.recv() {
            Response::Hello { protocol, .. } => assert_eq!(protocol, PROTOCOL_VERSION),
            other => panic!("expected hello, got {other:?}"),
        }
        worker
    }

    fn send(&mut self, request: &Request, payload: &[u8]) {
        write_frame(self.stdin.as_mut().unwrap(), request, payload).unwrap();
    }

    fn recv(&mut self) -> Response {
        read_frame::<Response>(&mut self.stdout).unwrap().unwrap().0
    }

    fn load(&mut self) {
        let model = std::env::var("LOMI_STT_TEST_MODEL").unwrap();
        self.send(
            &Request::Load {
                model,
                vad: std::env::var("LOMI_STT_TEST_VAD").ok(),
                acceleration: Acceleration::Auto,
            },
            &[],
        );
        match self.recv() {
            Response::Loaded { .. } => {}
            other => panic!("expected loaded, got {other:?}"),
        }
    }

    fn begin(&mut self) {
        self.send(
            &Request::Begin {
                language: None,
                vocabulary: Vec::new(),
                filter_silence: true,
            },
            &[],
        );
    }

    fn feed(&mut self, pcm: &[f32]) {
        for chunk in pcm.chunks(1600) {
            self.send(&Request::Feed, &pcm_to_bytes(chunk));
        }
    }
}

fn enabled() -> bool {
    std::env::var_os("LOMI_STT_TEST_MODEL").is_some()
}

fn wav_16k_mono(path: &str) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels, spec.bits_per_sample),
        (16_000, 1, 16)
    );
    reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect()
}

#[test]
fn exits_promptly_when_stdin_closes() {
    let mut worker = Worker::spawn();
    drop(worker.stdin.take());
    let started = Instant::now();
    while worker.child.try_wait().unwrap().is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "worker did not exit"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn protocol_garbage_terminates_worker() {
    let mut worker = Worker::spawn();
    worker
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&[0xff; 16])
        .unwrap();
    assert!(!worker.child.wait().unwrap().success());
}

#[test]
fn begin_without_model_is_an_input_error() {
    let mut worker = Worker::spawn();
    worker.begin();
    assert!(matches!(worker.recv(), Response::Error { .. }));
}

#[test]
fn silence_gives_empty_result() {
    if !enabled() || std::env::var_os("LOMI_STT_TEST_VAD").is_none() {
        return;
    }
    let mut worker = Worker::spawn();
    worker.load();
    worker.begin();
    worker.feed(&vec![0.0; 16_000 * 2]);
    worker.send(&Request::Finish, &[]);
    match worker.recv() {
        Response::Result {
            text, speech_ms, ..
        } => {
            assert!(text.is_empty());
            assert_eq!(speech_ms, 0);
        }
        other => panic!("expected result, got {other:?}"),
    }
}

#[test]
fn transcribes_speech_and_cancel_discards_audio() {
    let (true, Ok(wav)) = (enabled(), std::env::var("LOMI_STT_TEST_WAV")) else {
        return;
    };
    let pcm = wav_16k_mono(&wav);
    let mut worker = Worker::spawn();
    worker.load();

    worker.begin();
    worker.feed(&pcm);
    worker.send(&Request::Cancel, &[]);
    assert_eq!(worker.recv(), Response::Cancelled);

    worker.begin();
    worker.feed(&pcm);
    worker.send(&Request::Finish, &[]);
    match worker.recv() {
        Response::Result {
            text, speech_ms, ..
        } => {
            assert!(!text.is_empty());
            assert!(speech_ms > 0);
        }
        other => panic!("expected result, got {other:?}"),
    }
}
