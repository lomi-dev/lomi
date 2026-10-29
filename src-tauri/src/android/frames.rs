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

#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
