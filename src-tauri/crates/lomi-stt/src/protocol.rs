//! Framing and messages between Lomi (parent) and `lomi-stt-worker` (child).
//!
//! Transport is the worker's stdin/stdout. Each frame is
//! `[u32 LE header length][JSON header][u32 LE payload length][payload]`.
//! Only `Feed` carries a payload (PCM as f32 LE); every other frame sends an
//! empty one. Both lengths are capped before anything is allocated, so a
//! corrupt or hostile stream cannot make either side allocate unbounded memory.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// Bumped on any incompatible change; a mismatch in `Hello` kills the worker.
pub const PROTOCOL_VERSION: u32 = 1;
/// A header is a small JSON object; results are at most a few KiB of text.
pub const MAX_HEADER: usize = 256 * 1024;
/// One `Feed` frame: 2 s of 16 kHz f32 audio. Senders use ~100 ms frames.
pub const MAX_PAYLOAD: usize = 2 * 16_000 * 4;

/// Parent to worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    /// Load (or replace) the ASR model and optional VAD model.
    Load {
        model: String,
        vad: Option<String>,
        acceleration: Acceleration,
    },
    /// Start a dictation session; audio follows as `Feed` frames.
    Begin {
        language: Option<String>,
        vocabulary: Vec<String>,
        filter_silence: bool,
    },
    /// Payload: little-endian f32 PCM, 16 kHz mono.
    Feed,
    /// No more audio for this session; reply with `Result`.
    Finish,
    /// Drop the session audio and any pending result.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Acceleration {
    Auto,
    Cpu,
}

/// Worker to parent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Response {
    Hello {
        protocol: u32,
        engine: String,
        devices: Vec<DeviceInfo>,
    },
    Loaded {
        backend: String,
        languages: Vec<String>,
        vocabulary: bool,
        long_form: bool,
    },
    /// VAD edge, used for the auto-stop timer and the UI.
    Speech {
        active: bool,
    },
    Result {
        text: String,
        audio_ms: u64,
        speech_ms: u64,
    },
    Cancelled,
    Error {
        kind: ErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub kind: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The accelerator failed; the parent retries the same audio on CPU once.
    GpuFault,
    OutOfMemory,
    /// The model file is missing, corrupt or unsupported.
    Model,
    /// A request arrived in the wrong state or with invalid fields.
    Input,
    /// The CPU lacks an instruction set this build needs.
    UnsupportedCpu,
    Internal,
}

pub fn write_frame<T: Serialize>(
    out: &mut impl Write,
    header: &T,
    payload: &[u8],
) -> io::Result<()> {
    let header =
        serde_json::to_vec(header).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if header.len() > MAX_HEADER || payload.len() > MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame too large",
        ));
    }
    out.write_all(&(header.len() as u32).to_le_bytes())?;
    out.write_all(&header)?;
    out.write_all(&(payload.len() as u32).to_le_bytes())?;
    out.write_all(payload)?;
    out.flush()
}

/// Reads one frame. `Ok(None)` is a clean EOF before the first byte.
pub fn read_frame<T: for<'de> Deserialize<'de>>(
    input: &mut impl Read,
) -> io::Result<Option<(T, Vec<u8>)>> {
    let Some(header_len) = read_len(input, MAX_HEADER, true)? else {
        return Ok(None);
    };
    let mut header = vec![0; header_len];
    input.read_exact(&mut header)?;
    let header = serde_json::from_slice(&header)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let payload_len = read_len(input, MAX_PAYLOAD, false)?.unwrap_or(0);
    let mut payload = vec![0; payload_len];
    input.read_exact(&mut payload)?;
    Ok(Some((header, payload)))
}

fn read_len(input: &mut impl Read, max: usize, eof_ok: bool) -> io::Result<Option<usize>> {
    let mut bytes = [0u8; 4];
    let mut filled = 0;
    while filled < 4 {
        match input.read(&mut bytes[filled..]) {
            Ok(0) if filled == 0 && eof_ok => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_le_bytes(bytes) as usize;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    Ok(Some(len))
}

pub fn pcm_to_bytes(pcm: &[f32]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

pub fn bytes_to_pcm(bytes: &[u8]) -> io::Result<Vec<f32>> {
    let (frames, rest) = bytes.as_chunks::<4>();
    if !rest.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PCM payload is not f32-aligned",
        ));
    }
    Ok(frames.iter().map(|b| f32::from_le_bytes(*b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de>>(
        value: &T,
        payload: &[u8],
    ) -> (T, Vec<u8>) {
        let mut buf = Vec::new();
        write_frame(&mut buf, value, payload).unwrap();
        read_frame(&mut buf.as_slice()).unwrap().unwrap()
    }

    #[test]
    fn requests_and_responses_roundtrip() {
        let begin = Request::Begin {
            language: Some("pl".into()),
            vocabulary: vec!["Lomi".into()],
            filter_silence: true,
        };
        assert_eq!(roundtrip(&begin, &[]).0, begin);
        let error = Response::Error {
            kind: ErrorKind::GpuFault,
            message: "lost".into(),
        };
        assert_eq!(roundtrip(&error, &[]).0, error);
    }

    #[test]
    fn feed_payload_roundtrips_pcm() {
        let pcm = [0.0, -1.0, 0.5, f32::MIN_POSITIVE];
        let (request, payload) = roundtrip(&Request::Feed, &pcm_to_bytes(&pcm));
        assert_eq!(request, Request::Feed);
        assert_eq!(bytes_to_pcm(&payload).unwrap(), pcm);
    }

    #[test]
    fn several_frames_read_in_order_then_clean_eof() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &Request::Finish, &[]).unwrap();
        write_frame(&mut buf, &Request::Cancel, &[]).unwrap();
        let mut input = buf.as_slice();
        assert_eq!(
            read_frame::<Request>(&mut input).unwrap().unwrap().0,
            Request::Finish
        );
        assert_eq!(
            read_frame::<Request>(&mut input).unwrap().unwrap().0,
            Request::Cancel
        );
        assert!(read_frame::<Request>(&mut input).unwrap().is_none());
    }

    #[test]
    fn oversized_lengths_are_rejected_before_allocating() {
        let mut header = ((MAX_HEADER + 1) as u32).to_le_bytes().to_vec();
        header.extend_from_slice(b"{}");
        assert_eq!(
            read_frame::<Request>(&mut header.as_slice())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        let mut buf = Vec::new();
        let json = br#"{"type":"feed"}"#;
        buf.extend_from_slice(&(json.len() as u32).to_le_bytes());
        buf.extend_from_slice(json);
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            read_frame::<Request>(&mut buf.as_slice())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(write_frame(&mut Vec::new(), &Request::Feed, &vec![0; MAX_PAYLOAD + 1]).is_err());
    }

    #[test]
    fn truncated_frame_and_garbage_header_are_errors() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &Request::Finish, &[]).unwrap();
        buf.truncate(buf.len() - 2);
        assert!(read_frame::<Request>(&mut buf.as_slice()).is_err());
        let mut junk = 5u32.to_le_bytes().to_vec();
        junk.extend_from_slice(b"hello");
        junk.extend_from_slice(&0u32.to_le_bytes());
        assert!(read_frame::<Request>(&mut junk.as_slice()).is_err());
    }

    #[test]
    fn misaligned_pcm_is_rejected() {
        assert!(bytes_to_pcm(&[0, 0, 0]).is_err());
    }
}
