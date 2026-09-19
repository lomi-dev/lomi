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
type Sink = Box<dyn Fn(Vec<u8>) -> Result<(), String> + Send + Sync>;

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
            .ok_or("Android stream epochs exhausted. Restart SimpleBench.")?;
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
                        stream(connection, size, generation_bytes, epoch, &entry, &manager, sink).await
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

async fn stream(
    connection: Arc<Connection>,
    size: Size,
    generation: [u8; 16],
    epoch: u64,
    entry: &Entry,
    manager: &Weak<Manager>,
    sink: Sink,
) -> Result<(), String> {
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
        let mut last_sent = tokio::time::Instant::now() - Duration::from_secs(1);
        let mut sleeping = None;
        loop {
            receiver
                .changed()
                .await
                .map_err(|_| "Android image source ended")?;
            tokio::time::sleep_until(last_sent + Duration::from_micros(33_334)).await;
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
            let packet = packet(&frame, size, generation, epoch, sequence)?;
            drop(frame);
            let bytes = packet.len() as u64;
            *entry
                .pending
                .lock()
                .map_err(|_| "Android frame acknowledgement failed")? = Some(sequence);
            sink(packet)?;
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
            last_sent = tokio::time::Instant::now();
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

fn packet(
    frame: &Image,
    size: Size,
    generation: [u8; 16],
    epoch: u64,
    sequence: u64,
) -> Result<Vec<u8>, String> {
    let (width, height, rotation) = validate(frame, size)?;
    let length = (HEADER + frame.image.len()).max(MIN_PACKET);
    let mut packet = Vec::with_capacity(length);
    packet.extend_from_slice(b"SBAP");
    packet.extend_from_slice(&4u32.to_le_bytes());
    packet.extend_from_slice(&generation);
    for value in [epoch, sequence, frame.timestamp_us] {
        packet.extend_from_slice(&value.to_le_bytes());
    }
    for value in [width, height, rotation as u32, 0, frame.image.len() as u32] {
        packet.extend_from_slice(&value.to_le_bytes());
    }
    packet.extend_from_slice(&frame.image);
    packet.resize(length, 0);
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
        let trial = PathBuf::from(std::env::var_os("SIMPLEBENCH_ANDROID_PROBE_DIRECTORY").unwrap())
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
                Box::new(move |packet| {
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
