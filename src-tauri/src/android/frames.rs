use super::{
    events::Event,
    manager::Manager,
    rpc::{Connection, MAX_PIXELS},
    runtime::{DeviceRuntime, Phase},
    storage::valid_id,
};
use crate::android_protocol::{Image, ImageFormat};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tokio::sync::{watch, Notify};

const HEADER: usize = 68;
const MIN_PACKET: usize = 1024;
const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
const FRAME_PERIOD: Duration = Duration::from_micros(33_334);
pub(super) type Delivery = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
pub(super) type Sink = Box<dyn Fn(Vec<u8>) -> Delivery + Send + Sync>;

pub(super) fn immediate(
    send: impl Fn(Vec<u8>) -> Result<(), String> + Send + Sync + 'static,
) -> Sink {
    Box::new(move |packet| {
        let result = send(packet);
        Box::pin(std::future::ready(result))
    })
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}
impl Size {
    fn validate(self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 1280
            || self.height > 1280
            || self
                .width
                .checked_mul(self.height)
                .is_none_or(|n| n > MAX_PIXELS)
        {
            return Err("Android stream dimensions exceed the supported display budget".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub device_id: String,
    pub generation: String,
    pub epoch: u64,
    pub phase: &'static str,
    pub error: Option<String>,
    pub grpc_frames: u64,
    pub grpc_bytes: u64,
    pub ipc_frames: u64,
    /// Logical uncompressed packet bytes, including header/padding, before negotiated encoding.
    pub ipc_bytes: u64,
}

#[derive(Default)]
struct Counters {
    grpc_frames: AtomicU64,
    grpc_bytes: AtomicU64,
    ipc_frames: AtomicU64,
    ipc_bytes: AtomicU64,
}

struct Entry {
    status: Mutex<Status>,
    counters: Counters,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    pending: Mutex<Option<u64>>,
    acknowledged: Notify,
}
impl Entry {
    fn status(&self) -> Result<Status, String> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Android stream state failed")?
            .clone();
        status.grpc_frames = self.counters.grpc_frames.load(Ordering::Relaxed);
        status.grpc_bytes = self.counters.grpc_bytes.load(Ordering::Relaxed);
        status.ipc_frames = self.counters.ipc_frames.load(Ordering::Relaxed);
        status.ipc_bytes = self.counters.ipc_bytes.load(Ordering::Relaxed);
        Ok(status)
    }
    fn publish(&self, manager: &Weak<Manager>, phase: &'static str, error: Option<String>) {
        if let Ok(mut status) = self.status.lock() {
            status.phase = phase;
            status.error = error;
        }
        if let (Some(manager), Ok(status)) = (manager.upgrade(), self.status()) {
            manager.emit(Event::Stream(status));
        }
    }
    fn ack(&self, sequence: u64) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "Android frame acknowledgement failed")?;
        if *pending == Some(sequence) {
            *pending = None;
            self.acknowledged.notify_one();
        }
        Ok(())
    }
    async fn wait_ack(&self) -> Result<(), String> {
        tokio::time::timeout(ACK_TIMEOUT, async {
            loop {
                let notification = self.acknowledged.notified();
                if self
                    .pending
                    .lock()
                    .map_err(|_| "Android frame acknowledgement failed")?
                    .is_none()
                {
                    return Ok(());
                }
                notification.await;
            }
        })
        .await
        .map_err(|_| "Android image acknowledgement timed out. Reconnect the panel.".to_string())?
    }
}

#[derive(Default)]
struct Core {
    next_epoch: u64,
    devices: BTreeMap<String, Arc<Entry>>,
}

/// Exactly one source and one binary IPC destination per device. Replacements
/// await the previous owner's completion, including a still-connecting source.
#[derive(Default)]
pub struct Streams {
    core: Mutex<Core>,
}
impl Streams {
    pub fn subscribe(
        &self,
        manager: &Arc<Manager>,
        runtime: DeviceRuntime,
        generation: String,
        size: Size,
        sink: Sink,
    ) -> Result<u64, String> {
        self.subscribe_with_encoding(manager, runtime, generation, size, sink, Encoding::Raw)
    }

    pub(super) fn subscribe_with_encoding(
        &self,
        manager: &Arc<Manager>,
        runtime: DeviceRuntime,
        generation: String,
        size: Size,
        sink: Sink,
        encoding: Encoding,
    ) -> Result<u64, String> {
        let options = PacketOptions {
            size,
            encoding: if cfg!(target_os = "linux") {
                encoding
            } else {
                Encoding::Raw
            },
        };
        size.validate()?;
        let generation_bytes = generation_bytes(&generation)?;
        let status = runtime.status();
        if status.generation.as_ref() != Some(&generation) || status.phase != Phase::Running {
            return Err("The Android instance changed. Reconnect the panel.".into());
        }
        let mut core = self.core.lock().map_err(|_| "Android streams failed")?;
        let epoch = core
            .next_epoch
            .checked_add(1)
            .filter(|n| *n <= MAX_SAFE_INTEGER)
            .ok_or("Android stream epochs exhausted. Restart Lomi.")?;
        core.next_epoch = epoch;
        let previous = core.devices.get(&status.device_id).map(|entry| {
            entry.cancel.send_replace(true);
            entry.done.clone()
        });
        let (cancel, mut cancelled) = watch::channel(false);
        let (finished, done) = watch::channel(false);
        let entry = Arc::new(Entry {
            status: Mutex::new(Status {
                device_id: status.device_id.clone(),
                generation: generation.clone(),
                epoch,
                phase: "connecting",
                error: None,
                grpc_frames: 0,
                grpc_bytes: 0,
                ipc_frames: 0,
                ipc_bytes: 0,
            }),
            counters: Counters::default(),
            cancel,
            done,
            pending: Mutex::new(None),
            acknowledged: Notify::new(),
        });
        core.devices.insert(status.device_id, entry.clone());
        let manager = Arc::downgrade(manager);
        tauri::async_runtime::spawn(async move {
            // Even a cancelled replacement must await its predecessor; otherwise
            // a third subscription could overtake an older live source.
            if let Some(mut previous) = previous {
                wait_done(&mut previous).await;
            }
            let result = if *cancelled.borrow() {
                Ok(())
            } else {
                entry.publish(&manager, "connecting", None);
                let mut status = runtime.subscribe();
                tokio::select! {
                    _ = cancelled.changed() => Ok(()),
                    _ = async {
                        loop {
                            let running = { let status = status.borrow_and_update(); status.generation.as_ref() == Some(&generation) && status.phase == Phase::Running };
                            if !running || status.changed().await.is_err() { break; }
                        }
                    } => Ok(()),
                    result = async {
                        let connection = runtime.connection().await?;
                        stream(connection, options, generation_bytes, epoch, &entry, &manager, sink).await
                    } => result,
                }
            };
            if let Ok(mut pending) = entry.pending.lock() {
                *pending = None;
            }
            match result {
                Ok(()) => entry.publish(&manager, "hidden", None),
                Err(error) => entry.publish(&manager, "disconnected", Some(error)),
            }
            finished.send_replace(true);
        });
        Ok(epoch)
    }

    pub fn ack(
        &self,
        device: &str,
        generation: &str,
        epoch: u64,
        sequence: u64,
    ) -> Result<(), String> {
        let core = self.core.lock().map_err(|_| "Android streams failed")?;
        if let Some(entry) = core.devices.get(device) {
            let status = entry
                .status
                .lock()
                .map_err(|_| "Android stream state failed")?;
            if status.generation == generation && status.epoch == epoch {
                entry.ack(sequence)?;
            }
        }
        Ok(())
    }

    pub async fn unsubscribe(
        &self,
        device: &str,
        generation: &str,
        epoch: u64,
    ) -> Result<(), String> {
        let completion = {
            let core = self.core.lock().map_err(|_| "Android streams failed")?;
            if let Some(entry) = core.devices.get(device) {
                let status = entry
                    .status
                    .lock()
                    .map_err(|_| "Android stream state failed")?;
                if status.generation == generation && status.epoch == epoch {
                    entry.cancel.send_replace(true);
                    Some(entry.done.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(mut completion) = completion {
            wait_done(&mut completion).await;
        }
        Ok(())
    }

    pub fn hide_all(&self) {
        if let Ok(core) = self.core.lock() {
            for entry in core.devices.values() {
                entry.cancel.send_replace(true);
            }
        }
    }

    pub fn statuses(&self) -> Result<Vec<Status>, String> {
        self.core
            .lock()
            .map_err(|_| "Android streams failed")?
            .devices
            .values()
            .map(|entry| entry.status())
            .collect()
    }
}

async fn wait_done(completion: &mut watch::Receiver<bool>) {
    loop {
        if *completion.borrow_and_update() || completion.changed().await.is_err() {
            return;
        }
    }
}

#[derive(Clone, Copy)]
struct PacketOptions {
    size: Size,
    encoding: Encoding,
}

#[derive(Clone, Copy)]
pub(super) enum Encoding {
    Raw,
    Deflate,
    Rle32,
}

async fn stream(
    connection: Arc<Connection>,
    options: PacketOptions,
    generation: [u8; 16],
    epoch: u64,
    entry: &Entry,
    manager: &Weak<Manager>,
    sink: Sink,
) -> Result<(), String> {
    let PacketOptions { size, encoding } = options;
    let request = connection.request(
        "streamScreenshot",
        ImageFormat {
            format: 1,
            width: size.width,
            height: size.height,
            ..Default::default()
        },
    )?;
    let mut input = connection
        .client()
        .stream_screenshot(request)
        .await
        .map_err(|error| format!("Android image connection failed: {error}. Reconnect the panel."))?
        .into_inner();
    let (latest, mut receiver) = watch::channel::<Option<Image>>(None);
    let read = async {
        loop {
            let Some(frame) = input.message().await.map_err(|error| {
                format!("Android image stream ended: {error}. Reconnect the panel.")
            })?
            else {
                return Err::<(), String>(
                    "Android image stream closed. Reconnect the panel.".into(),
                );
            };
            validate(&frame, size)?;
            entry.counters.grpc_frames.fetch_add(1, Ordering::Relaxed);
            entry
                .counters
                .grpc_bytes
                .fetch_add(frame.image.len() as u64, Ordering::Relaxed);
            // Image.image is prost::bytes::Bytes. This replaces the sole waiting
            // frame and never clones its pixel allocation.
            latest.send_replace(Some(frame));
        }
    };
    let send = async {
        let mut sequence = 0;
        let mut pacing = tokio::time::interval(FRAME_PERIOD);
        pacing.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sleeping = None;
        loop {
            receiver
                .changed()
                .await
                .map_err(|_| "Android image source ended")?;
            // Fixed phase avoids adding serialization, UI delivery, and ACK
            // time to every period. Skip missed ticks and send only the latest frame.
            pacing.tick().await;
            let Some(frame) = receiver.borrow_and_update().clone() else {
                continue;
            };
            sequence += 1;
            if sequence > MAX_SAFE_INTEGER {
                return Err::<(), String>(
                    "Android frame sequence exhausted. Reconnect the panel.".into(),
                );
            }
            let is_sleeping = frame.image.is_empty();
            let packet = match encoding {
                #[cfg(target_os = "linux")]
                Encoding::Rle32 => rle32_packet(&frame, size, generation, epoch, sequence)?,
                #[cfg(target_os = "linux")]
                Encoding::Deflate => compressed_packet(&frame, size, generation, epoch, sequence)?,
                _ => packet(&frame, size, generation, epoch, sequence)?,
            };
            // Counters describe logical raw bytes regardless of negotiated wire encoding.
            let bytes = (HEADER + frame.image.len()).max(MIN_PACKET) as u64;
            drop(frame);
            *entry
                .pending
                .lock()
                .map_err(|_| "Android frame acknowledgement failed")? = Some(sequence);
            sink(packet).await?;
            entry.counters.ipc_frames.fetch_add(1, Ordering::Relaxed);
            entry.counters.ipc_bytes.fetch_add(bytes, Ordering::Relaxed);
            if sleeping != Some(is_sleeping) {
                sleeping = Some(is_sleeping);
                entry.publish(
                    manager,
                    if is_sleeping { "sleeping" } else { "streaming" },
                    None,
                );
            }
            entry.wait_ack().await?;
        }
    };
    // Dropping either sibling cancels the tonic source immediately, including
    // while the guest screen is idle and would not produce another frame.
    tokio::try_join!(read, send).map(|_: ((), ())| ())
}

fn generation_bytes(generation: &str) -> Result<[u8; 16], String> {
    if !valid_id(generation) {
        return Err("Invalid Android process generation".into());
    }
    let value = generation.replace('-', "");
    let mut bytes = [0; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
            .map_err(|_| "Invalid Android process generation")?;
    }
    Ok(bytes)
}

fn validate(frame: &Image, requested: Size) -> Result<(u32, u32, i32), String> {
    let format = frame
        .format
        .as_ref()
        .ok_or("Android frame has no format. Reconnect the panel.")?;
    let rotation = format.rotation.as_ref().map_or(0, |value| value.rotation);
    if !(0..=3).contains(&rotation) {
        return Err("Invalid Android frame orientation".into());
    }
    if format.width == 0 && format.height == 0 && frame.image.is_empty() {
        return Ok((0, 0, rotation));
    }
    let pixels = format
        .width
        .checked_mul(format.height)
        .filter(|n| *n > 0 && *n <= MAX_PIXELS)
        .ok_or("Android frame exceeds the pixel limit")?;
    if format.width > requested.width
        || format.height > requested.height
        || format.format != 1
        || frame.image.len() != pixels as usize * 4
    {
        return Err("Android returned an invalid RGBA frame or ignored source scaling. Reconnect the panel.".into());
    }
    Ok((format.width, format.height, rotation))
}

fn packet_header(
    frame: &Image,
    size: Size,
    generation: [u8; 16],
    epoch: u64,
    sequence: u64,
    capacity: usize,
) -> Result<Vec<u8>, String> {
    let (width, height, rotation) = validate(frame, size)?;
    let mut packet = Vec::with_capacity(capacity);
    packet.extend_from_slice(b"SBAP");
    packet.extend_from_slice(&4u32.to_le_bytes());
    packet.extend_from_slice(&generation);
    for value in [epoch, sequence, frame.timestamp_us] {
        packet.extend_from_slice(&value.to_le_bytes());
    }
    for value in [width, height, rotation as u32, 0, frame.image.len() as u32] {
        packet.extend_from_slice(&value.to_le_bytes());
    }
    Ok(packet)
}

fn finish_raw_packet(mut packet: Vec<u8>, image: &[u8]) -> Vec<u8> {
    packet.truncate(HEADER);
    packet.reserve(image.len());
    packet.extend_from_slice(image);
    packet.resize((HEADER + image.len()).max(MIN_PACKET), 0);
    packet
}

fn packet(
    frame: &Image,
    size: Size,
    generation: [u8; 16],
    epoch: u64,
    sequence: u64,
) -> Result<Vec<u8>, String> {
    Ok(finish_raw_packet(
        packet_header(
            frame,
            size,
            generation,
            epoch,
            sequence,
            (HEADER + frame.image.len()).max(MIN_PACKET),
        )?,
        &frame.image,
    ))
}

#[cfg(any(target_os = "linux", test))]
fn lossless_packet_header(
    frame: &Image,
    size: Size,
    generation: [u8; 16],
    epoch: u64,
    sequence: u64,
) -> Result<Vec<u8>, String> {
    let packet = packet_header(frame, size, generation, epoch, sequence, HEADER)?;
    if epoch == 0 || epoch > MAX_SAFE_INTEGER || sequence == 0 || sequence > MAX_SAFE_INTEGER {
        return Err("Invalid Android packet identity for lossless encoding".into());
    }
    if !frame.image.is_empty() {
        let format = frame.format.as_ref().ok_or("Android frame has no format")?;
        Size {
            width: format.width,
            height: format.height,
        }
        .validate()?;
    }
    Ok(packet)
}

/// Append bounded pixel runs directly from borrowed RGBA; false selects raw fallback.
#[cfg(any(target_os = "linux", test))]
fn encode_rle32(rgba: &[u8], out: &mut Vec<u8>) -> bool {
    if rgba.is_empty() || rgba.len() % 4 != 0 || rgba.len() / 4 > MAX_PIXELS as usize {
        return false;
    }
    fn append(out: &mut Vec<u8>, bytes: &[u8], limit: usize) -> bool {
        let Some(needed) = out.len().checked_add(bytes.len()) else {
            return false;
        };
        if needed > limit {
            return false;
        }
        if out.capacity() < needed {
            let capacity = needed.max(out.capacity().saturating_mul(2)).min(limit);
            out.reserve_exact(capacity - out.len());
        }
        out.extend_from_slice(bytes);
        true
    }
    let limit = out.len() + rgba.len() * 7 / 8;
    let count = rgba.len() / 4;
    let pixel = |i: usize| &rgba[i * 4..i * 4 + 4];
    let repeat_at = |i: usize| i + 3 < count && (1..4).all(|j| pixel(i) == pixel(i + j));
    let mut i = 0;
    while i < count {
        if repeat_at(i) {
            let start = i;
            i += 4;
            while i < count && pixel(i) == pixel(start) {
                i += 1;
            }
            let token = (0x8000_0000 | (i - start) as u32).to_le_bytes();
            if !append(out, &token, limit) || !append(out, pixel(start), limit) {
                return false;
            }
        } else {
            let start = i;
            i += 1;
            while i < count && !repeat_at(i) {
                i += 1;
            }
            let token = ((i - start) as u32).to_le_bytes();
            if !append(out, &token, limit) || !append(out, &rgba[start * 4..i * 4], limit) {
                return false;
            }
        }
    }
    true
}

#[cfg(any(target_os = "linux", test))]
fn rle32_packet(
    frame: &Image,
    size: Size,
    generation: [u8; 16],
    epoch: u64,
    sequence: u64,
) -> Result<Vec<u8>, String> {
    let mut packet = lossless_packet_header(frame, size, generation, epoch, sequence)?;
    let logical_length = (HEADER + frame.image.len()).max(MIN_PACKET);
    if !encode_rle32(&frame.image, &mut packet) || packet.len().max(MIN_PACKET) >= logical_length {
        return Ok(finish_raw_packet(packet, &frame.image));
    }
    let encoded_length = packet.len() - HEADER;
    packet[60..64].copy_from_slice(&2u32.to_le_bytes());
    packet[64..68].copy_from_slice(&(encoded_length as u32).to_le_bytes());
    packet.resize(packet.len().max(MIN_PACKET), 0);
    Ok(packet)
}

/// Encode directly from validated borrowed RGBA into the final header/payload Vec.
/// No full raw packet is staged when the negotiated Linux encoding is worthwhile.
#[cfg(any(target_os = "linux", test))]
fn compressed_packet(
    frame: &Image,
    size: Size,
    generation: [u8; 16],
    epoch: u64,
    sequence: u64,
) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut packet = lossless_packet_header(frame, size, generation, epoch, sequence)?;
    if frame.image.is_empty() {
        return Ok(finish_raw_packet(packet, &frame.image));
    }
    let length = frame.image.len();
    let logical_length = (HEADER + length).max(MIN_PACKET);
    // The encoder appends after the completed header, borrowing the source pixels.
    let mut encoder = flate2::write::ZlibEncoder::new(packet, flate2::Compression::fast());
    encoder
        .write_all(&frame.image)
        .map_err(|error| error.to_string())?;
    packet = encoder.finish().map_err(|error| error.to_string())?;
    let encoded_length = packet.len() - HEADER;
    let wire_length = packet.len().max(MIN_PACKET);
    if encoded_length == 0 || encoded_length * 8 > length * 7 || wire_length >= logical_length {
        return Ok(finish_raw_packet(packet, &frame.image));
    }
    packet[60..64].copy_from_slice(&1u32.to_le_bytes());
    packet[64..68].copy_from_slice(&(encoded_length as u32).to_le_bytes());
    packet.resize(wire_length, 0);
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::android::{
        auth,
        devices::{Action, Draft},
        manager::Android,
        storage::{Gpu, Hardware},
    };

    #[test]
    fn frame_validation_bounds_pixels_orientation_and_empty_display() {
        let size = Size {
            width: 720,
            height: 1280,
        };
        let mut frame = Image {
            format: Some(ImageFormat {
                format: 1,
                width: 2,
                height: 3,
                ..Default::default()
            }),
            image: vec![3; 24].into(),
            ..Default::default()
        };
        let generation = generation_bytes("01234567-89ab-4def-8abc-0123456789ab").unwrap();
        let bytes = packet(&frame, size, generation, 12, 34).unwrap();
        assert_eq!(bytes.len(), MIN_PACKET);
        assert_eq!(&bytes[8..24], &generation);
        assert_eq!(u64::from_le_bytes(bytes[24..32].try_into().unwrap()), 12);
        assert_eq!(u64::from_le_bytes(bytes[32..40].try_into().unwrap()), 34);
        assert_eq!(&bytes[HEADER..HEADER + 24], &[3; 24]);
        frame.format.as_mut().unwrap().width = u32::MAX;
        assert!(packet(&frame, size, generation, 12, 35).is_err());
        frame.format.as_mut().unwrap().width = 0;
        frame.format.as_mut().unwrap().height = 0;
        assert!(packet(&frame, size, generation, 12, 35).is_err());
        frame.image = Default::default();
        assert!(packet(&frame, size, generation, 12, 35).is_ok());
        frame.format.as_mut().unwrap().rotation = Some(crate::android_protocol::Rotation {
            rotation: 4,
            ..Default::default()
        });
        assert!(packet(&frame, size, generation, 12, 35).is_err());
        assert!(Size {
            width: 1280,
            height: 1280
        }
        .validate()
        .is_err());
        assert!(Size {
            width: 0,
            height: 720
        }
        .validate()
        .is_err());
    }

    fn compression_source(width: u32, height: u32, image: Vec<u8>) -> Image {
        Image {
            format: Some(ImageFormat {
                format: 1,
                width,
                height,
                ..Default::default()
            }),
            image: image.into(),
            timestamp_us: 123456789,
            ..Default::default()
        }
    }

    fn encode_source(frame: &Image, compressed: bool) -> Result<Vec<u8>, String> {
        let encode = if compressed {
            compressed_packet
        } else {
            packet
        };
        encode(
            frame,
            Size {
                width: 720,
                height: 1280,
            },
            [17; 16],
            12,
            34,
        )
    }

    #[test]
    fn lossless_packets_round_trip_preserve_identity_and_zero_padding() {
        use std::io::Read;
        for (width, height) in [(64, 64), (720, 1280)] {
            let image: Vec<_> = (0..width * height * 4)
                .map(|index| (index % 4 * 50) as u8)
                .collect();
            let frame = compression_source(width, height, image);
            let source_allocation = frame.image.as_ptr();
            let raw = encode_source(&frame, false).unwrap();
            let encoded = encode_source(&frame, true).unwrap();
            assert_eq!(frame.image.as_ptr(), source_allocation);
            assert_eq!(&encoded[..60], &raw[..60]);
            assert_eq!(u32::from_le_bytes(encoded[60..64].try_into().unwrap()), 1);
            let count = u32::from_le_bytes(encoded[64..68].try_into().unwrap()) as usize;
            assert_eq!(encoded.len(), (HEADER + count).max(MIN_PACKET));
            assert!(encoded[HEADER + count..].iter().all(|byte| *byte == 0));
            assert!(encoded.len() < raw.len());
            assert!(count * 8 <= frame.image.len() * 7);
            let mut decoded = Vec::new();
            flate2::read::ZlibDecoder::new(&encoded[HEADER..HEADER + count])
                .take(frame.image.len() as u64 + 1)
                .read_to_end(&mut decoded)
                .unwrap();
            assert_eq!(decoded.as_slice(), frame.image.as_ref());
            // The final compressed Vec was not preallocated as a full RGBA packet.
            assert!(encoded.capacity() < frame.image.len());
        }
    }

    #[test]
    fn lossless_encoding_keeps_incompressible_tiny_and_sleeping_packets_raw() {
        let mut random = 0x123456789abcdef0u64;
        let image = (0..128 * 128 * 4)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random as u8
            })
            .collect();
        for frame in [
            compression_source(128, 128, image),
            compression_source(2, 2, vec![0; 16]),
            compression_source(0, 0, vec![]),
        ] {
            assert_eq!(
                encode_source(&frame, true).unwrap(),
                encode_source(&frame, false).unwrap()
            );
        }
    }

    #[test]
    fn lossless_encoding_rejects_invalid_source_before_encoding() {
        let source = compression_source(64, 64, vec![0; 64 * 64 * 4]);
        let size = Size {
            width: 720,
            height: 1280,
        };
        for (epoch, sequence) in [
            (0, 1),
            (1, 0),
            (MAX_SAFE_INTEGER + 1, 1),
            (1, MAX_SAFE_INTEGER + 1),
        ] {
            assert!(compressed_packet(&source, size, [17; 16], epoch, sequence).is_err());
        }
        for (width, height) in [
            (1281, 1),
            (0, 64),
            (64, 0),
            (1280, 1280),
            (u32::MAX, u32::MAX),
        ] {
            let mut invalid = source.clone();
            let format = invalid.format.as_mut().unwrap();
            format.width = width;
            format.height = height;
            assert!(encode_source(&invalid, true).is_err());
        }
        let mut invalid = source.clone();
        invalid.format = None;
        assert!(encode_source(&invalid, true).is_err());
        let mut invalid = source.clone();
        invalid.format.as_mut().unwrap().format = 2;
        assert!(encode_source(&invalid, true).is_err());
        let mut invalid = source.clone();
        invalid.format.as_mut().unwrap().rotation = Some(crate::android_protocol::Rotation {
            rotation: 4,
            ..Default::default()
        });
        assert!(encode_source(&invalid, true).is_err());
        let mut invalid = source;
        invalid.image = vec![0; 4].into();
        assert!(encode_source(&invalid, true).is_err());
        let over_edge = compression_source(1281, 1, vec![0; 1281 * 4]);
        assert!(compressed_packet(
            &over_edge,
            Size {
                width: 1281,
                height: 1
            },
            [17; 16],
            1,
            1
        )
        .is_err());
    }

    fn decode_rle_payload(bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let token = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
            i += 4;
            let count = (token & 0x7fff_ffff) as usize;
            assert!(count > 0);
            if token & 0x8000_0000 != 0 {
                for _ in 0..count {
                    output.extend_from_slice(&bytes[i..i + 4]);
                }
                i += 4;
            } else {
                output.extend_from_slice(&bytes[i..i + count * 4]);
                i += count * 4;
            }
        }
        output
    }

    #[test]
    fn rle32_packets_preserve_mixed_pixels_identity_and_padding() {
        for (width, height) in [(64, 64), (720, 1280)] {
            let mut image = [31, 128, 255, 9].repeat((width * height) as usize);
            for (i, pixel) in image.chunks_exact_mut(4).take(5).enumerate() {
                pixel.copy_from_slice(&(i as u32).to_le_bytes());
            }
            let frame = compression_source(width, height, image);
            let source = frame.image.as_ptr();
            let raw = encode_source(&frame, false).unwrap();
            let encoded = rle32_packet(
                &frame,
                Size {
                    width: 720,
                    height: 1280,
                },
                [17; 16],
                12,
                34,
            )
            .unwrap();
            assert_eq!(frame.image.as_ptr(), source);
            assert_eq!(&encoded[..60], &raw[..60]);
            assert_eq!(u32::from_le_bytes(encoded[60..64].try_into().unwrap()), 2);
            let length = u32::from_le_bytes(encoded[64..68].try_into().unwrap()) as usize;
            assert_eq!(encoded.len(), (HEADER + length).max(MIN_PACKET));
            assert!(encoded[HEADER + length..].iter().all(|byte| *byte == 0));
            assert_eq!(
                decode_rle_payload(&encoded[HEADER..HEADER + length]),
                frame.image.as_ref()
            );
            assert!(length * 8 <= frame.image.len() * 7);
            assert!(encoded.capacity() < frame.image.len());
        }
    }

    #[test]
    fn rle32_falls_back_raw_without_trying_deflate() {
        let image = (0..64 * 64)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [0, 0, 0, 255]
                } else {
                    [255, 0, 0, 255]
                }
            })
            .collect();
        let alternating = compression_source(64, 64, image);
        assert_eq!(
            u32::from_le_bytes(
                encode_source(&alternating, true).unwrap()[60..64]
                    .try_into()
                    .unwrap()
            ),
            1
        );
        for frame in [
            alternating,
            compression_source(2, 2, vec![0; 16]),
            compression_source(0, 0, vec![]),
        ] {
            let encoded = rle32_packet(
                &frame,
                Size {
                    width: 720,
                    height: 1280,
                },
                [17; 16],
                12,
                34,
            )
            .unwrap();
            assert_eq!(encoded, encode_source(&frame, false).unwrap());
        }
    }

    #[test]
    fn rle32_threshold_and_token_rules_are_bounded() {
        let mut exact: Vec<u8> = (0..4u32).flat_map(u32::to_le_bytes).collect();
        exact.extend([9, 8, 7, 6].repeat(4));
        let mut output = Vec::new();
        assert!(encode_rle32(&exact, &mut output));
        assert_eq!(output.len() * 8, exact.len() * 7);
        assert_eq!(decode_rle_payload(&output), exact);
        let mut too_large: Vec<u8> = (0..5u32).flat_map(u32::to_le_bytes).collect();
        too_large.extend([9, 8, 7, 6].repeat(4));
        assert!(!encode_rle32(&too_large, &mut Vec::new()));
        for invalid in [vec![], vec![0; 3], vec![0; MAX_PIXELS as usize * 4 + 4]] {
            assert!(!encode_rle32(&invalid, &mut Vec::new()));
        }
    }

    #[test]
    fn rle32_rejects_invalid_source_and_packet_identity() {
        let mut frame = compression_source(64, 64, vec![0; 64 * 64 * 4]);
        let size = Size {
            width: 720,
            height: 1280,
        };
        for (epoch, sequence) in [
            (0, 1),
            (1, 0),
            (MAX_SAFE_INTEGER + 1, 1),
            (1, MAX_SAFE_INTEGER + 1),
        ] {
            assert!(rle32_packet(&frame, size, [17; 16], epoch, sequence).is_err());
        }
        frame.image = vec![0; 4].into();
        assert!(rle32_packet(&frame, size, [17; 16], 1, 1).is_err());
        let over_edge = compression_source(1281, 1, vec![0; 1281 * 4]);
        assert!(rle32_packet(
            &over_edge,
            Size {
                width: 1281,
                height: 1
            },
            [17; 16],
            1,
            1
        )
        .is_err());
    }

    #[tokio::test]
    async fn delivery_waits_for_completion_and_propagates_failure() {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let receiver = Arc::new(Mutex::new(Some(receiver)));
        let sink: Sink = Box::new(move |_| {
            let receiver = receiver.lock().unwrap().take().unwrap();
            Box::pin(async move { receiver.await.map_err(|e| e.to_string())? })
        });
        let mut delivery = sink(vec![0; MIN_PACKET]);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), &mut delivery)
                .await
                .is_err()
        );
        sender.send(Err("UI post failed".to_string())).unwrap();
        assert_eq!(delivery.await.unwrap_err(), "UI post failed");
        assert_eq!(
            immediate(|_| Err("channel closed".into()))(vec![])
                .await
                .unwrap_err(),
            "channel closed"
        );
    }

    #[tokio::test]
    async fn late_ack_cannot_release_another_epoch_or_generation() {
        let streams = Streams::default();
        let id = auth::new_id().unwrap();
        let generation = auth::new_id().unwrap();
        let (cancel, _) = watch::channel(false);
        let (_, done) = watch::channel(false);
        let entry = Arc::new(Entry {
            status: Mutex::new(Status {
                device_id: id.clone(),
                generation: generation.clone(),
                epoch: 7,
                phase: "streaming",
                error: None,
                grpc_frames: 0,
                grpc_bytes: 0,
                ipc_frames: 0,
                ipc_bytes: 0,
            }),
            counters: Counters::default(),
            cancel,
            done,
            pending: Mutex::new(Some(1)),
            acknowledged: Notify::new(),
        });
        streams
            .core
            .lock()
            .unwrap()
            .devices
            .insert(id.clone(), entry.clone());
        streams.ack(&id, &generation, 6, 1).unwrap();
        streams.ack(&id, &auth::new_id().unwrap(), 7, 1).unwrap();
        streams.ack(&id, &generation, 7, 2).unwrap();
        assert_eq!(*entry.pending.lock().unwrap(), Some(1));
        streams.ack(&id, &generation, 7, 1).unwrap();
        entry.wait_ack().await.unwrap();
        *entry.pending.lock().unwrap() = Some(2);
        streams.ack(&id, &generation, 7, 1).unwrap();
        assert_eq!(*entry.pending.lock().unwrap(), Some(2));
    }

    #[tokio::test]
    #[ignore = "Boots a real isolated managed AVD and tests the production authenticated source/ACK lifecycle"]
    async fn native_managed_stream_input_and_cancellation() {
        use std::{fs, path::PathBuf};
        let trial = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap())
            .canonicalize()
            .unwrap();
        let installation: serde_json::Value = serde_json::from_slice(
            &fs::read(trial.join("evidence/native-managed-installation.json")).unwrap(),
        )
        .unwrap();
        let root = PathBuf::from(installation["root"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        assert!(root.starts_with(&trial));
        let android = Android::default();
        let manager = android.get(root.clone()).unwrap();
        manager.test_adb_port.store(15047, Ordering::Release);
        assert!(std::net::TcpListener::bind(("127.0.0.1", 15047)).is_ok());
        let revision = manager
            .directory
            .lock()
            .unwrap()
            .devices()
            .unwrap()
            .revision;
        let mut created_id = None;
        let result: Result<serde_json::Value, String> = async {
            manager.installer.manage(&manager, Action::Create { expected_revision: revision, draft: Draft { name: "Native stream trial".into(), image: "system-images;android-36;default;arm64-v8a".into(), profile: "small_phone".into(), hardware: Hardware { ram_mib: 2560, cpu_count: 2, data_gib: 6, gpu: Gpu::Host, quick_boot: false } } })?;
            manager.installer.settle(false).await?;
            let operation = manager.installer.progress().ok_or("Missing create result")?;
            if operation.phase != crate::android::installer::Phase::Succeeded { return Err(format!("Creation failed: {operation:?}")); }
            let id = operation.device_id.ok_or("No created device")?;
            created_id = Some(id.clone());
            let started = manager.start(&id).await?;
            let generation = started.generation.ok_or("Missing process generation")?;
            let runtime = manager.runtime(&id, &generation)?;
            runtime.install_apk(generation.clone(), fs::File::open(trial.join("development-tools/input-build/input-test.apk")).map_err(|e| e.to_string())?).await?;
            let record: serde_json::Value = serde_json::from_slice(&fs::read(root.join("runtime").join(format!("{id}.json"))).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            let guest = crate::android::adb::Guest { server: crate::android::adb::Server { port: 15047 }, console_port: record["consolePort"].as_u64().ok_or("Missing native console port")? as u16, device_id: id.clone(), generation_key: record["generationKey"].as_str().ok_or("Missing native transport nonce")?.into() };
            let open_guest = guest.clone();
            tokio::task::spawn_blocking(move || open_guest.native_input_fixture(true)).await.map_err(|e| e.to_string())??;
            tokio::time::sleep(Duration::from_secs(1)).await;
            let restarted_guest = guest.clone();
            tokio::task::spawn_blocking(move || -> Result<(), String> {
                for _ in 0..4 {
                    restarted_guest.enable_input(&Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
                    restarted_guest.text("ping", "")?;
                }
                Ok(())
            }).await.map_err(|e| e.to_string())??;
            use crate::android::input::{Router, Request, Event as Input, TextAction, TouchPhase};
            let focus = Router::submit(manager.clone(), id.clone(), generation.clone(), Request::Focus { view_id: auth::new_id()? }).await?;
            let lease = focus.lease.ok_or("No native input lease")?;
            let input_events = [
                Input::Text { action: TextAction::Commit, text: "Zażółć gęślą jaźń".into() },
                Input::Text { action: TextAction::Compose, text: "に".into() },
                Input::Text { action: TextAction::Compose, text: "日本".into() },
                Input::Text { action: TextAction::Commit, text: "日本語".into() },
                Input::Paste { text: " — wklejone żółć".into() },
            ];
            for (index, event) in input_events.into_iter().enumerate() {
                Router::submit(manager.clone(), id.clone(), generation.clone(), Request::Send { lease: lease.clone(), sequence: index as u64 + 1, event }).await?;
            }
            let read_guest = guest.clone();
            let screen = tokio::task::spawn_blocking(move || read_guest.native_input_fixture(false)).await.map_err(|e| e.to_string())??;
            if !screen.contains("text=\"Zażółć gęślą jaźń日本語 — wklejone żółć\"") { return Err(format!("Production Unicode/IME/Paste did not reach the editor: {screen}")); }
            Router::submit(manager.clone(), id.clone(), generation.clone(), Request::Send { lease: lease.clone(), sequence: 6, event: Input::Touch { identifier: 0, x: 360, y: 900, phase: TouchPhase::Down } }).await?;
            Router::submit(manager.clone(), id.clone(), generation.clone(), Request::Blur { lease: lease.clone() }).await?;
            let read_guest = guest.clone();
            let screen = tokio::task::spawn_blocking(move || read_guest.native_input_fixture(false)).await.map_err(|e| e.to_string())??;
            if !screen.contains("action 1") { return Err("Blur did not release the native touch".into()); }
            if Router::submit(manager.clone(), id.clone(), generation.clone(), Request::Send { lease, sequence: 7, event: Input::Text { action: TextAction::Commit, text: "STALE INPUT".into() } }).await.is_ok() { return Err("Input with a released focus lease was accepted".into()); }
            let size = Size { width: 360, height: 640 };
            let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
            let make_sink = || -> Sink {
                let send = send.clone();
                immediate(move |packet| {
                    if packet.len() < HEADER || &packet[..4] != b"SBAP" || packet[4] != 4 { return Err("Invalid production frame packet".into()); }
                    let epoch = u64::from_le_bytes(packet[24..32].try_into().unwrap());
                    let sequence = u64::from_le_bytes(packet[32..40].try_into().unwrap());
                    let width = u32::from_le_bytes(packet[48..52].try_into().unwrap());
                    let height = u32::from_le_bytes(packet[52..56].try_into().unwrap());
                    if width > 360 || height > 640 { return Err("Source did not scale frames".into()); }
                    send.send((epoch, sequence, width, height)).map_err(|e| e.to_string())
                })
            };
            let first = manager.streams.subscribe(&manager, runtime.clone(), generation.clone(), size, make_sink())?;
            let old = tokio::time::timeout(Duration::from_secs(10), receive.recv()).await.map_err(|_| "No first native frame")?.ok_or("Frame channel ended")?;
            if old.0 != first { return Err("Unexpected first epoch".into()); }
            let first_entry = manager.streams.core.lock().unwrap().devices[&id].clone();
            let mut done = first_entry.done.clone();
            tokio::time::timeout(Duration::from_secs(4), wait_done(&mut done)).await.map_err(|_| "Missing ACK did not stop the source")?;
            if !first_entry.status()?.error.is_some_and(|e| e.contains("acknowledgement timed out")) { return Err("Missing ACK produced the wrong failure".into()); }
            let second = manager.streams.subscribe(&manager, runtime.clone(), generation.clone(), size, make_sink())?;
            let fresh = tokio::time::timeout(Duration::from_secs(10), receive.recv()).await.map_err(|_| "No reconnected frame")?.ok_or("Frame channel ended")?;
            if fresh.0 != second || second == first { return Err("Reconnect reused its epoch".into()); }
            manager.streams.ack(&id, &generation, first, old.1)?;
            let second_entry = manager.streams.core.lock().unwrap().devices[&id].clone();
            if *second_entry.pending.lock().unwrap() != Some(fresh.1) { return Err("Old ACK released the new frame".into()); }
            manager.streams.ack(&id, &generation, second, fresh.1)?;
            manager.streams.unsubscribe(&id, &generation, first).await?;
            if *second_entry.cancel.borrow() { return Err("Old unsubscribe cancelled the new source".into()); }
            let third = manager.streams.subscribe(&manager, runtime.clone(), generation.clone(), size, make_sink())?;
            let fourth = manager.streams.subscribe(&manager, runtime.clone(), generation.clone(), size, make_sink())?;
            loop {
                let frame = tokio::time::timeout(Duration::from_secs(10), receive.recv()).await.map_err(|_| "No frame after overlapping reconnect")?.ok_or("Frame channel ended")?;
                if frame.0 == fourth { manager.streams.ack(&id, &generation, fourth, frame.1)?; break; }
                if frame.0 != second && frame.0 != third { return Err("Unexpected replacement epoch".into()); }
            }
            let current = manager.streams.core.lock().unwrap().devices[&id].clone();
            manager.streams.hide_all();
            let mut done = current.done.clone();
            tokio::time::timeout(Duration::from_secs(1), wait_done(&mut done)).await.map_err(|_| "Hide did not cancel the native source")?;
            let before = current.status()?;
            tokio::time::sleep(Duration::from_millis(150)).await;
            if current.status()?.grpc_frames != before.grpc_frames || runtime.status().phase != Phase::Running { return Err("Hide failed to stop frames while preserving the phone".into()); }
            let fifth = manager.streams.subscribe(&manager, runtime.clone(), generation.clone(), size, make_sink())?;
            let last = manager.streams.core.lock().unwrap().devices[&id].clone();
            manager.stop(&id, false).await?;
            let mut done = last.done.clone();
            tokio::time::timeout(Duration::from_secs(1), wait_done(&mut done)).await.map_err(|_| "Stop retained its source")?;
            Ok(serde_json::json!({ "host": "macos-arm64", "source": "production Rust authenticated gRPC", "requested": [360,640], "firstFrame": [old.2,old.3], "epochs": [first,second,third,fourth,fifth], "ackTimeout": true, "staleAckRejected": true, "staleUnsubscribeRejected": true, "replacementSerialized": true, "hideCancelledSource": true, "hideKeptProcess": true, "stopCancelledSource": true, "bundledImeInstalledAtBoot": true, "imeServiceRestarts": 4, "apkInstalledByRuntime": true, "directPolish": true, "japaneseComposition": true, "explicitUnicodePaste": true, "blurReleasedTouch": true, "staleInputRejected": true, "tauriChannelAndCanvas": "not part of this Rust-only test" }))
        }.await;
        if let Some(id) = created_id {
            let _ = manager.stop(&id, false).await;
            if manager
                .statuses()
                .unwrap()
                .iter()
                .any(|s| s.device_id == id && s.process_alive)
            {
                manager.stop(&id, true).await.unwrap();
            }
            let revision = manager
                .directory
                .lock()
                .unwrap()
                .devices()
                .unwrap()
                .revision;
            manager
                .installer
                .manage(
                    &manager,
                    Action::Delete {
                        expected_revision: revision,
                        device_id: id,
                        confirmation: "Native stream trial".into(),
                    },
                )
                .unwrap();
            manager.installer.settle(false).await.unwrap();
        }
        manager.stop_private_adb_fixture().unwrap();
        let report = result.unwrap();
        fs::write(
            trial.join("evidence/native-managed-frames.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("{report}");
    }
}
