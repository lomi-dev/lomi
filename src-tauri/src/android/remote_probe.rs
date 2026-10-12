//! G1/R04 in-memory source prototype, excluded from ordinary builds.
//! This is not remote admission, a network transport, or production Streams.
use super::{
    rpc::MAX_PIXELS,
    runtime::{DeviceRuntime, Phase},
};
use crate::android_protocol::{Image, ImageFormat};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{watch, Notify},
    time::Instant,
};

const ACK_TIMEOUT: Duration = Duration::from_secs(2);
const HEALTH_INTERVAL: Duration = Duration::from_secs(10);
const RPC_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_CONSUMERS: usize = 3;
const MAX_JPEG_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Local,
    Remote,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fence {
    pub generation: String,
    pub config: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub fence: Fence,
    pub receipt: u64,
    pub geometry_epoch: u64,
    pub width: u32,
    pub height: u32,
    pub rotation: i32,
    pub sleeping: bool,
    pub bytes: Arc<[u8]>,
}
#[derive(Clone, Debug)]
pub(crate) struct Delivery {
    pub epoch: u64,
    pub frame: Arc<Frame>,
}
struct Consumer {
    kind: Kind,
    epoch: u64,
    waiting: Option<Arc<Frame>>,
    pending: Option<(u64, Instant)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Health {
    Connecting,
    Ready,
    Sleeping,
    NotReady,
    Failed,
    Ended,
}
struct Core {
    fence: Fence,
    size: (u32, u32),
    receipt: u64,
    geometry_epoch: u64,
    epoch: u64,
    raw: Option<Arc<Frame>>,
    jpeg: Option<Arc<Frame>>,
    consumers: BTreeMap<String, Consumer>,
    health: Health,
    confirmed: Option<Instant>,
    stream_calls: u64,
    stream_frames: u64,
    unary_calls: u64,
    encodes: u64,
}
impl Core {
    fn new(fence: Fence, size: (u32, u32)) -> Result<Self, String> {
        if size.0 == 0
            || size.1 == 0
            || size.0 > 1280
            || size.1 > 1280
            || size.0.checked_mul(size.1).is_none_or(|n| n > MAX_PIXELS)
        {
            return Err("Invalid probe source budget".into());
        }
        Ok(Self {
            fence,
            size,
            receipt: 0,
            geometry_epoch: 0,
            epoch: 0,
            raw: None,
            jpeg: None,
            consumers: BTreeMap::new(),
            health: Health::Connecting,
            confirmed: None,
            stream_calls: 0,
            stream_frames: 0,
            unary_calls: 0,
            encodes: 0,
        })
    }
    fn join(&mut self, id: String, kind: Kind) -> Result<u64, String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid memory sink ID".into());
        }
        if self.health == Health::Ended || self.health == Health::Failed {
            return Err("Source ended".into());
        }
        if self.consumers.len() >= MAX_CONSUMERS && !self.consumers.contains_key(&id) {
            return Err("Probe consumer capacity".into());
        }
        if self
            .consumers
            .iter()
            .any(|(key, c)| key != &id && c.kind == Kind::Local)
            && kind == Kind::Local
        {
            return Err("One local sink only".into());
        }
        if kind == Kind::Remote
            && self
                .consumers
                .iter()
                .filter(|(key, c)| *key != &id && c.kind == Kind::Remote)
                .count()
                >= 2
        {
            return Err("Two remote sinks only".into());
        }
        self.epoch = self.epoch.checked_add(1).ok_or("Epoch exhausted")?;
        let waiting = if kind == Kind::Local {
            self.raw.clone()
        } else {
            self.jpeg.clone()
        };
        self.consumers.insert(
            id,
            Consumer {
                kind,
                epoch: self.epoch,
                waiting,
                pending: None,
            },
        );
        Ok(self.epoch)
    }
    fn matches(&self, fence: &Fence) -> bool {
        self.fence == *fence && !matches!(self.health, Health::Ended | Health::Failed)
    }
    fn publish(
        &mut self,
        fence: &Fence,
        watermark: Option<u64>,
        image: Image,
    ) -> Result<Option<Arc<Frame>>, String> {
        if !self.matches(fence) || watermark.is_some_and(|w| w != self.receipt) {
            return Ok(None);
        }
        let format = image.format.ok_or("Missing frame format")?;
        let rotation = format.rotation.map_or(0, |r| r.rotation);
        if !(0..=3).contains(&rotation) {
            return Err("Invalid rotation".into());
        }
        let sleeping = format.width == 0 && format.height == 0 && image.image.is_empty();
        if !sleeping
            && (format.format != 1
                || format.width > self.size.0
                || format.height > self.size.1
                || format
                    .width
                    .checked_mul(format.height)
                    .filter(|n| *n > 0 && *n <= MAX_PIXELS)
                    .is_none_or(|n| image.image.len() != n as usize * 4))
        {
            return Err("Invalid bounded RGBA sample".into());
        }
        self.receipt = self.receipt.checked_add(1).ok_or("Receipt exhausted")?;
        // Guest seq is deliberately unused: unary seq=0 is not stream ordering.
        let geometry_changed = self.raw.as_ref().is_none_or(|old| {
            (old.width, old.height, old.rotation, old.sleeping)
                != (format.width, format.height, rotation, sleeping)
        });
        if geometry_changed {
            self.geometry_epoch = self
                .geometry_epoch
                .checked_add(1)
                .ok_or("Geometry exhausted")?;
            self.jpeg = None;
        }
        let frame = Arc::new(Frame {
            fence: fence.clone(),
            receipt: self.receipt,
            geometry_epoch: self.geometry_epoch,
            width: format.width,
            height: format.height,
            rotation,
            sleeping,
            bytes: image.image.to_vec().into(),
        });
        self.raw = Some(frame.clone());
        self.confirmed = Some(Instant::now());
        self.health = if sleeping {
            Health::Sleeping
        } else {
            Health::Ready
        };
        for c in self.consumers.values_mut() {
            if c.kind == Kind::Local || sleeping {
                c.waiting = Some(frame.clone());
            } else if geometry_changed {
                c.waiting = None;
            }
        }
        if sleeping {
            self.jpeg = Some(frame.clone());
        }
        Ok(Some(frame))
    }
    fn encoded(&mut self, frame: Arc<Frame>) {
        if self.health != Health::Ready
            || !self.matches(&frame.fence)
            || frame.geometry_epoch != self.geometry_epoch
            || frame.receipt > self.receipt
            || self
                .jpeg
                .as_ref()
                .is_some_and(|old| old.receipt >= frame.receipt)
        {
            return;
        }
        self.encodes += 1;
        self.jpeg = Some(frame.clone());
        for c in self
            .consumers
            .values_mut()
            .filter(|c| c.kind == Kind::Remote)
        {
            c.waiting = Some(frame.clone());
        }
    }
    fn remove(&mut self, id: &str, fence: &Fence, epoch: u64) {
        if self.fence == *fence && self.consumers.get(id).is_some_and(|c| c.epoch == epoch) {
            self.consumers.remove(id);
        }
    }
    fn ack(&mut self, id: &str, fence: &Fence, epoch: u64, receipt: u64) {
        if self.fence != *fence {
            return;
        }
        if let Some(c) = self.consumers.get_mut(id) {
            if c.epoch == epoch && c.pending.is_some_and(|(r, _)| r == receipt) {
                c.pending = None;
            }
        }
    }
    fn take(&mut self, id: &str, fence: &Fence, epoch: u64, now: Instant) -> Option<Delivery> {
        if !self.matches(fence) {
            return None;
        }
        let c = self.consumers.get_mut(id)?;
        if c.epoch != epoch || c.pending.is_some() {
            return None;
        }
        let frame = c.waiting.take()?;
        c.pending = Some((frame.receipt, now));
        Some(Delivery {
            epoch: c.epoch,
            frame,
        })
    }
    fn expire(&mut self, now: Instant) {
        self.consumers.retain(|_, c| {
            !c.pending
                .is_some_and(|(_, at)| now.saturating_duration_since(at) >= ACK_TIMEOUT)
        });
    }
    fn clear(&mut self, health: Health) {
        self.geometry_epoch = self.geometry_epoch.saturating_add(1);
        self.raw = None;
        self.jpeg = None;
        self.confirmed = None;
        self.health = health;
        for c in self.consumers.values_mut() {
            c.waiting = None;
            c.pending = None;
        }
    }
}

/// A single owner must be constructed for the isolated DeviceRuntime generation.
/// No commands or production callers expose this experimental owner.
struct SourceSlot(String);
fn source_slots() -> &'static Mutex<BTreeSet<String>> {
    static SLOTS: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(BTreeSet::new()))
}
impl SourceSlot {
    fn acquire(key: String) -> Result<Self, String> {
        if !source_slots().lock().unwrap().insert(key.clone()) {
            return Err("Probe source already owned for this device generation".into());
        }
        Ok(Self(key))
    }
}
impl Drop for SourceSlot {
    fn drop(&mut self) {
        source_slots().lock().unwrap().remove(&self.0);
    }
}

pub(crate) struct Source {
    core: Arc<Mutex<Core>>,
    demand: Arc<Notify>,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
impl Source {
    pub(crate) fn new(
        runtime: DeviceRuntime,
        fence: Fence,
        size: (u32, u32),
    ) -> Result<Self, String> {
        let status = runtime.status();
        if status.phase != Phase::Running || status.generation.as_ref() != Some(&fence.generation) {
            return Err("Runtime generation is not running".into());
        }
        let core = Arc::new(Mutex::new(Core::new(fence.clone(), size)?));
        let slot = SourceSlot::acquire(format!("{}:{}", status.device_id, fence.generation))?;
        let demand = Arc::new(Notify::new());
        let (cancel, cancelled) = watch::channel(false);
        let (finished, done) = watch::channel(false);
        let owner = Self {
            core: core.clone(),
            demand: demand.clone(),
            cancel,
            done,
        };
        tokio::spawn(async move {
            let _slot = slot;
            let result = run(runtime, fence, core.clone(), demand, cancelled).await;
            let mut c = core.lock().unwrap();
            let health = if c.health == Health::NotReady {
                Health::NotReady
            } else if result.is_ok() {
                Health::Ended
            } else {
                Health::Failed
            };
            c.clear(health);
            drop(c);
            drop(_slot);
            finished.send_replace(true);
        });
        Ok(owner)
    }
    pub(crate) fn join(&self, id: &str, kind: Kind) -> Result<u64, String> {
        if *self.done.borrow() {
            return Err("Source ended; construct a new owner".into());
        }
        let epoch = self.core.lock().unwrap().join(id.into(), kind)?;
        self.demand.notify_one();
        Ok(epoch)
    }
    pub(crate) fn take(&self, id: &str, fence: &Fence, epoch: u64) -> Option<Delivery> {
        self.core
            .lock()
            .unwrap()
            .take(id, fence, epoch, Instant::now())
    }
    pub(crate) fn ack(&self, id: &str, fence: &Fence, epoch: u64, receipt: u64) {
        self.core.lock().unwrap().ack(id, fence, epoch, receipt);
    }
    pub(crate) fn remove(&self, id: &str, fence: &Fence, epoch: u64) {
        self.core.lock().unwrap().remove(id, fence, epoch);
        self.demand.notify_one();
    }
    pub(crate) fn hide_local(&self) {
        self.core
            .lock()
            .unwrap()
            .consumers
            .retain(|_, c| c.kind != Kind::Local);
        self.demand.notify_one();
    }
    pub(crate) async fn close(&self) {
        self.cancel.send_replace(true);
        let mut done = self.done.clone();
        while !*done.borrow_and_update() && done.changed().await.is_ok() {}
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}

struct CappedJpeg(Vec<u8>);
impl std::io::Write for CappedJpeg {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_JPEG_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(
                "JPEG exceeds 512 KiB prototype budget",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode(raw: Arc<Frame>) -> Result<Arc<Frame>, String> {
    let mut jpeg = CappedJpeg(Vec::with_capacity(MAX_JPEG_BYTES));
    let rgb: Vec<u8> = raw
        .bytes
        .chunks_exact(4)
        .flat_map(|pixel| pixel[..3].iter().copied())
        .collect();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 70)
        .encode(&rgb, raw.width, raw.height, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(Frame {
        bytes: jpeg.0.into(),
        ..(*raw).clone()
    }))
}
fn rpc_error(error: &tonic::Status) -> Result<Health, String> {
    if error.code() == tonic::Code::FailedPrecondition {
        Ok(Health::NotReady)
    } else {
        Err(error.to_string())
    }
}
async fn run(
    runtime: DeviceRuntime,
    fence: Fence,
    core: Arc<Mutex<Core>>,
    demand: Arc<Notify>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    // Wait for demand before opening any screenshot RPC. This owner never starts/stops a phone.
    while core.lock().unwrap().consumers.is_empty() {
        tokio::select! { _ = demand.notified() => {}, _ = cancel.changed() => return Ok(()) }
    }
    let status_before = runtime.status();
    if status_before.phase != Phase::Running
        || status_before.generation.as_ref() != Some(&fence.generation)
    {
        return Ok(());
    }
    let connection = runtime.connection().await?;
    let validity_fence = fence.clone();
    let valid = Arc::new(move || {
        let s = runtime.status();
        Ok(s.phase == Phase::Running && s.generation.as_ref() == Some(&validity_fence.generation))
    });
    run_connected(connection, fence, core, demand, cancel, valid).await
}

async fn run_connected(
    connection: Arc<super::rpc::Connection>,
    fence: Fence,
    core: Arc<Mutex<Core>>,
    demand: Arc<Notify>,
    mut cancel: watch::Receiver<bool>,
    valid: Arc<dyn Fn() -> Result<bool, String> + Send + Sync>,
) -> Result<(), String> {
    while core.lock().unwrap().consumers.is_empty() {
        tokio::select! { _ = demand.notified() => {}, _ = cancel.changed() => return Ok(()) }
    }
    if !valid()? {
        return Ok(());
    }
    let size = core.lock().unwrap().size;
    let format = ImageFormat {
        format: 1,
        width: size.0,
        height: size.1,
        ..Default::default()
    };
    core.lock().unwrap().stream_calls += 1;
    let mut stream_client = connection.client();
    let mut stream = tokio::select! {
        _ = cancel.changed() => return Ok(()),
        reply = tokio::time::timeout(RPC_TIMEOUT, stream_client.stream_screenshot(connection.request("streamScreenshot", format.clone())?)) => reply.map_err(|_| "Stream connect timeout")?.map_err(|e| e.to_string())?.into_inner(),
    };
    let (raw_send, mut raw_recv) = watch::channel::<Option<Arc<Frame>>>(None);
    let encoder_core = core.clone();
    let encoder_valid = valid.clone();
    let encoder = tokio::spawn(async move {
        let mut next_encode = Instant::now();
        while raw_recv.changed().await.is_ok() {
            tokio::time::sleep_until(next_encode).await;
            if matches!(
                encoder_core.lock().unwrap().health,
                Health::Ended | Health::Failed
            ) {
                break;
            }
            let Some(raw) = raw_recv.borrow_and_update().clone() else {
                continue;
            };
            if raw.sleeping {
                continue;
            }
            let jpeg = tokio::task::spawn_blocking(move || encode(raw))
                .await
                .map_err(|e| e.to_string())??;
            if !encoder_valid()? {
                return Ok(());
            }
            encoder_core.lock().unwrap().encoded(jpeg);
            next_encode = Instant::now() + Duration::from_millis(100);
        }
        Ok::<(), String>(())
    });
    // Exactly one coalesced unary request in flight. A receipt watermark fences late results.
    let mut probe: Option<tokio::task::JoinHandle<(u64, Result<Image, String>, bool)>> = None;
    let mut next_probe = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    let outcome = async {
        loop {
            if encoder.is_finished() { return Err("Shared JPEG encoder ended".into()); }
            if !valid()? { return Ok(()); }
            {
                let mut c = core.lock().unwrap();
                c.expire(Instant::now());
                if c.consumers.is_empty() { return Ok(()); }
            }
            if probe.as_ref().is_some_and(|p| p.is_finished()) {
                let (watermark, result, not_ready) = probe.take().unwrap().await.map_err(|e| e.to_string())?;
                if !valid()? { return Ok(()); }
                let mut c = core.lock().unwrap();
                if c.matches(&fence) && c.receipt == watermark {
                    if not_ready { c.clear(Health::NotReady); }
                    else {
                        let sample = c.publish(&fence, Some(watermark), result?)?;
                        if let Some(sample) = sample { raw_send.send_replace(Some(sample)); }
                    }
                }
            }
            let idle = core.lock().unwrap().confirmed.is_none_or(|at| at.elapsed() >= HEALTH_INTERVAL);
            if probe.is_none() && idle && Instant::now() >= next_probe {
                let watermark = { let mut c = core.lock().unwrap(); c.unary_calls += 1; c.receipt };
                let connection = connection.clone();
                let format = format.clone();
                probe = Some(tokio::spawn(async move {
                    let result = async {
                        let mut client = connection.client();
                        let response = tokio::time::timeout(RPC_TIMEOUT, client.get_screenshot(connection.request("getScreenshot", format)?)).await.map_err(|_| "Screenshot probe timeout".to_string())?;
                        match response {
                            Ok(response) => Ok((Some(response.into_inner()), false)),
                            Err(error) => { rpc_error(&error)?; Ok((None, true)) }
                        }
                    }.await;
                    match result { Ok((Some(image), _)) => (watermark, Ok(image), false), Ok((None, _)) => (watermark, Err("Not ready".into()), true), Err(e) => (watermark, Err(e), false) }
                }));
                next_probe = Instant::now() + HEALTH_INTERVAL;
            }
            tokio::select! {
                _ = cancel.changed() => return Ok(()),
                _ = demand.notified() => {},
                _ = tick.tick() => {},
                message = stream.message() => {
                    match message {
                        Ok(Some(image)) => { if !valid()? { return Ok(()); } let mut c = core.lock().unwrap(); c.stream_frames += 1; if let Some(raw) = c.publish(&fence, None, image)? { raw_send.send_replace(Some(raw)); } },
                        Ok(None) => return Err("Screenshot stream ended".into()),
                        Err(e) => { core.lock().unwrap().clear(rpc_error(&e)?); return Err("Screenshot stream not ready; reconnect prototype".into()); }
                    }
                }
            }
        }
    }.await;
    if let Some(probe) = probe {
        probe.abort();
        let _ = probe.await;
    }
    let health = if core.lock().unwrap().health == Health::NotReady {
        Health::NotReady
    } else if outcome.is_ok() {
        Health::Ended
    } else {
        Health::Failed
    };
    let settled = settle_encoder(core, raw_send, encoder, health).await;
    outcome.and(settled)
}

async fn settle_encoder(
    core: Arc<Mutex<Core>>,
    raw: watch::Sender<Option<Arc<Frame>>>,
    encoder: tokio::task::JoinHandle<Result<(), String>>,
    health: Health,
) -> Result<(), String> {
    // Fence publication first. A spawn_blocking job cannot be cancelled: retain
    // the source slot until its actual completion rather than aborting its waiter.
    core.lock().unwrap().clear(health);
    drop(raw);
    encoder.await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fence() -> Fence {
        Fence {
            generation: "fixture-generation".into(),
            config: 1,
        }
    }
    fn image(value: u8) -> Image {
        Image {
            format: Some(ImageFormat {
                format: 1,
                width: 2,
                height: 2,
                ..Default::default()
            }),
            image: vec![value; 16].into(),
            seq: 0,
            ..Default::default()
        }
    }
    fn core() -> Core {
        Core::new(fence(), (360, 640)).unwrap()
    }
    fn publish(c: &mut Core, value: u8) -> Arc<Frame> {
        c.publish(&fence(), None, image(value)).unwrap().unwrap()
    }

    #[test]
    fn independent_ack_timeout_and_latest_only_sinks() {
        let mut c = core();
        let local = c.join("local".into(), Kind::Local).unwrap();
        let a = c.join("a".into(), Kind::Remote).unwrap();
        let b = c.join("b".into(), Kind::Remote).unwrap();
        let now = Instant::now();
        let raw = publish(&mut c, 10);
        c.encoded(encode(raw).unwrap());
        let first = c.take("a", &fence(), a, now).unwrap();
        c.take("local", &fence(), local, now).unwrap();
        c.take("b", &fence(), b, now + Duration::from_secs(1))
            .unwrap();
        for value in 20..30 {
            let raw = publish(&mut c, value);
            c.encoded(encode(raw).unwrap());
        }
        assert!(c.take("a", &fence(), a, now).is_none());
        c.ack("local", &fence(), local, 1);
        let last = c
            .take("local", &fence(), local, now + Duration::from_secs(1))
            .unwrap();
        assert_eq!(last.frame.receipt, 11);
        assert_eq!(last.frame.bytes[0], 29);
        c.ack("b", &fence(), b, 1);
        assert_eq!(
            c.take("b", &fence(), b, now + Duration::from_secs(1))
                .unwrap()
                .frame
                .receipt,
            11
        );
        c.expire(now + ACK_TIMEOUT);
        assert!(!c.consumers.contains_key("a"));
        assert!(c.consumers.contains_key("local") && c.consumers.contains_key("b"));
        c.ack("a", &fence(), a, first.frame.receipt);
        assert_eq!(c.raw.as_ref().unwrap().receipt, 11);
        assert_eq!(c.jpeg.as_ref().unwrap().receipt, 11);
    }
    #[test]
    fn stale_ack_removal_config_and_generation_cannot_touch_replacement() {
        let mut c = core();
        let old = c.join("a".into(), Kind::Remote).unwrap();
        let raw = publish(&mut c, 1);
        c.encoded(encode(raw).unwrap());
        let epoch = c.join("a".into(), Kind::Remote).unwrap();
        c.take("a", &fence(), epoch, Instant::now()).unwrap();
        c.ack("a", &fence(), old, 1);
        c.remove("a", &fence(), old);
        let mut stale = fence();
        stale.config += 1;
        c.ack("a", &stale, epoch, 1);
        c.remove("a", &stale, epoch);
        stale.generation = "restarted".into();
        assert!(c.publish(&stale, None, image(3)).unwrap().is_none());
        assert_eq!(c.consumers["a"].pending.unwrap().0, 1);
        c.ack("a", &fence(), epoch, 1);
        c.remove("a", &fence(), epoch);
        assert!(c.consumers.is_empty());
        c.clear(Health::Ended);
        assert!(c.raw.is_none() && c.jpeg.is_none());
        assert!(c.publish(&fence(), None, image(3)).unwrap().is_none());
        assert!(c.join("a".into(), Kind::Remote).is_err());
    }
    #[test]
    fn stale_take_cannot_consume_replacement_subscription_frame() {
        let mut c = core();
        let old = c.join("a".into(), Kind::Remote).unwrap();
        let raw = publish(&mut c, 1);
        c.encoded(encode(raw).unwrap());
        let current = c.join("a".into(), Kind::Remote).unwrap();
        let waiting = c.consumers["a"].waiting.clone().unwrap();
        let now = Instant::now();
        assert!(c.take("a", &fence(), old, now).is_none());
        let mut stale = fence();
        stale.config += 1;
        assert!(c.take("a", &stale, current, now).is_none());
        assert!(c.consumers["a"].pending.is_none());
        assert!(Arc::ptr_eq(
            c.consumers["a"].waiting.as_ref().unwrap(),
            &waiting
        ));
        let delivery = c.take("a", &fence(), current, now).unwrap();
        assert_eq!(delivery.epoch, current);
        assert!(Arc::ptr_eq(&delivery.frame, &waiting));
        assert!(c.consumers["a"].waiting.is_none());
        assert_eq!(c.consumers["a"].pending.unwrap().0, delivery.frame.receipt);
    }
    #[test]
    fn cached_bootstrap_shares_encode_and_late_seq_zero_unary_is_rejected() {
        let mut c = core();
        let watermark = c.receipt;
        let raw = publish(&mut c, 10);
        c.encoded(encode(raw).unwrap());
        let shared = c.jpeg.clone().unwrap();
        c.join("a".into(), Kind::Remote).unwrap();
        c.join("b".into(), Kind::Remote).unwrap();
        assert!(Arc::ptr_eq(
            &c.consumers["a"].waiting.clone().unwrap(),
            &shared
        ));
        assert!(Arc::ptr_eq(
            &c.consumers["b"].waiting.clone().unwrap(),
            &shared
        ));
        assert_eq!(c.encodes, 1);
        assert!(c
            .publish(&fence(), Some(watermark), image(99))
            .unwrap()
            .is_none());
        assert_eq!(c.raw.as_ref().unwrap().bytes[0], 10);
        let current = c.receipt;
        assert!(c
            .publish(&fence(), Some(current), image(20))
            .unwrap()
            .is_some());
        assert_eq!(c.receipt, 2); // both images have guest seq=0
    }
    #[test]
    fn encoder_lag_accepts_same_geometry_but_rejects_changed_geometry() {
        let mut c = core();
        c.join("a".into(), Kind::Remote).unwrap();
        let earlier = publish(&mut c, 10);
        publish(&mut c, 20);
        c.encoded(encode(earlier.clone()).unwrap());
        assert_eq!(c.jpeg.as_ref().unwrap().receipt, 1);
        let rotated = Image {
            format: Some(ImageFormat {
                format: 1,
                width: 1,
                height: 2,
                ..Default::default()
            }),
            image: vec![30; 8].into(),
            ..Default::default()
        };
        c.publish(&fence(), None, rotated).unwrap();
        c.encoded(encode(earlier).unwrap());
        assert!(c.jpeg.is_none());
        assert!(c.consumers["a"].waiting.is_none());
    }
    #[test]
    fn sleeping_and_not_ready_clear_cached_pixels_without_extending_health() {
        let mut c = core();
        let a = c.join("a".into(), Kind::Remote).unwrap();
        let old = publish(&mut c, 1);
        c.encoded(encode(old.clone()).unwrap());
        let empty = Image {
            format: Some(ImageFormat::default()),
            ..Default::default()
        };
        c.publish(&fence(), None, empty).unwrap();
        c.encoded(encode(old).unwrap());
        assert_eq!(c.health, Health::Sleeping);
        assert!(c.jpeg.as_ref().unwrap().sleeping);
        assert!(c
            .take("a", &fence(), a, Instant::now())
            .unwrap()
            .frame
            .bytes
            .is_empty());
        assert_eq!(
            rpc_error(&tonic::Status::failed_precondition("not posted")).unwrap(),
            Health::NotReady
        );
        let in_flight = publish(&mut c, 20);
        c.clear(Health::NotReady);
        c.encoded(encode(in_flight.clone()).unwrap());
        assert!(c.raw.is_none() && c.jpeg.is_none() && c.confirmed.is_none());
        publish(&mut c, 30);
        c.encoded(encode(in_flight).unwrap());
        assert!(c.jpeg.is_none());
        assert!(rpc_error(&tonic::Status::unauthenticated("expired")).is_err());
    }
    #[test]
    fn source_slot_rejects_overlap_until_previous_owner_drops() {
        let key = "isolated-test-device:generation".to_string();
        let slot = SourceSlot::acquire(key.clone()).unwrap();
        assert!(SourceSlot::acquire(key.clone()).is_err());
        drop(slot);
        assert!(SourceSlot::acquire(key).is_ok());
    }
    #[tokio::test]
    async fn close_retains_source_slot_until_blocking_encode_really_settles() {
        let key = "settlement-test-device:generation".to_string();
        let core = Arc::new(Mutex::new(core()));
        let raw = publish(&mut core.lock().unwrap(), 10);
        let slot = SourceSlot::acquire(key.clone()).unwrap();
        let (raw_send, _) = watch::channel(Some(raw.clone()));
        let (release, blocked) = std::sync::mpsc::sync_channel::<()>(1);
        let (started, ready) = tokio::sync::oneshot::channel();
        let encoder_core = core.clone();
        let encoder = tokio::spawn(async move {
            let encoded = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                blocked.recv().unwrap();
                encode(raw)
            })
            .await
            .map_err(|e| e.to_string())??;
            encoder_core.lock().unwrap().encoded(encoded);
            Ok(())
        });
        ready.await.unwrap();
        let finish_core = core.clone();
        let finish = tokio::spawn(async move {
            let _slot = slot;
            settle_encoder(finish_core, raw_send, encoder, Health::Ended).await
        });
        tokio::task::yield_now().await;
        assert!(!finish.is_finished());
        assert!(SourceSlot::acquire(key.clone()).is_err());
        release.send(()).unwrap();
        finish.await.unwrap().unwrap();
        assert!(core.lock().unwrap().jpeg.is_none());
        assert!(SourceSlot::acquire(key).is_ok());
    }
    #[test]
    fn noisy_maximum_source_cannot_grow_jpeg_beyond_payload_budget() {
        use std::io::Write;
        let mut capped = CappedJpeg(Vec::with_capacity(MAX_JPEG_BYTES));
        capped.write_all(&vec![1; MAX_JPEG_BYTES]).unwrap();
        assert!(capped.write_all(&[2]).is_err());
        assert_eq!(capped.0.len(), MAX_JPEG_BYTES);
        let mut bytes = vec![0; MAX_PIXELS as usize * 4];
        let mut state = 123456789u32;
        for pixel in bytes.chunks_exact_mut(4) {
            for component in &mut pixel[..3] {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *component = state as u8;
            }
            pixel[3] = 255;
        }
        let mut c = Core::new(fence(), (720, 1280)).unwrap();
        let raw = c
            .publish(
                &fence(),
                None,
                Image {
                    format: Some(ImageFormat {
                        format: 1,
                        width: 720,
                        height: 1280,
                        ..Default::default()
                    }),
                    image: bytes.into(),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert!(encode(raw).unwrap_err().contains("512 KiB"));
    }
    #[test]
    fn consumer_capacity_and_source_budget_are_bounded() {
        let mut c = core();
        c.join("local".into(), Kind::Local).unwrap();
        assert!(c.join("second-local".into(), Kind::Local).is_err());
        c.join("a".into(), Kind::Remote).unwrap();
        c.join("b".into(), Kind::Remote).unwrap();
        assert!(c.join("c".into(), Kind::Remote).is_err());
        assert!(Core::new(fence(), (1280, 1280)).is_err());
        assert!(c
            .publish(
                &fence(),
                None,
                Image {
                    format: Some(ImageFormat {
                        format: 1,
                        width: u32::MAX,
                        height: 2,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            )
            .is_err());
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[tokio::test]
    #[ignore = "Explicit opt-in read-only attach to the running Lomi-owned AVD; no process/device lifecycle effects"]
    async fn native_attach_current_lomi_generation_three_sinks() {
        use super::super::{
            auth, avd, installation, process_identity::Identity, rpc::Connection, storage,
        };
        use std::{fs, io::Read, path::PathBuf};
        assert_eq!(
            std::env::var("LOMI_REMOTE_ANDROID_ATTACH").as_deref(),
            Ok("observe-current-generation")
        );
        let id = std::env::var("LOMI_REMOTE_ANDROID_DEVICE").unwrap();
        let generation = std::env::var("LOMI_REMOTE_ANDROID_GENERATION").unwrap();
        assert!(storage::valid_id(&id) && storage::valid_id(&generation));
        let root = crate::shell::home()
            .join("Library/Application Support/dev.lomi.desktop/android")
            .canonicalize()
            .unwrap();
        let path = installation::checked_path(&root, std::path::Path::new("runtime"))
            .unwrap()
            .join(format!("{id}.json"));
        let read_record = |path: &std::path::Path| -> Result<serde_json::Value, String> {
            if fs::symlink_metadata(path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("Symlink runtime record".into());
            }
            let mut bytes = Vec::new();
            fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(65537)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 65536 {
                return Err("Oversized runtime record".into());
            }
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        };
        let record = read_record(&path).unwrap();
        assert_eq!(record["deviceId"].as_str(), Some(id.as_str()));
        assert_eq!(record["generation"].as_str(), Some(generation.as_str()));
        assert_eq!(record["launching"], false);
        let process: Identity = serde_json::from_value(record["process"].clone()).unwrap();
        assert!(process.still_matches().unwrap());
        assert!(process
            .executable
            .starts_with(root.join("sdk/emulator").canonicalize().unwrap()));
        let discovery = avd::discovery(&root).unwrap();
        let published = avd::read_ini(&discovery.join(format!("pid_{}.ini", process.pid))).unwrap();
        let owned_avd =
            installation::checked_path(&root, &PathBuf::from("avd").join(format!("sb_{id}.avd")))
                .unwrap()
                .canonicalize()
                .unwrap();
        assert_eq!(published.get("avd.id"), Some(&format!("sb_{id}")));
        assert_eq!(
            published
                .get("avd.dir")
                .map(PathBuf::from)
                .unwrap()
                .canonicalize()
                .unwrap(),
            owned_avd
        );
        let port = u16::try_from(record["grpcPort"].as_u64().unwrap()).unwrap();
        let connection = Arc::new(
            Connection::register(
                &process,
                &discovery,
                port,
                Arc::new(auth::Authority::new(auth::new_id().unwrap()).unwrap()),
            )
            .await
            .unwrap(),
        );
        let fence = Fence {
            generation: generation.clone(),
            config: 1,
        };
        let slot = SourceSlot::acquire(format!("{id}:{generation}")).unwrap();
        let core = Arc::new(Mutex::new(Core::new(fence.clone(), (360, 640)).unwrap()));
        let demand = Arc::new(Notify::new());
        let (cancel, cancelled) = watch::channel(false);
        let (finished, done) = watch::channel(false);
        let source = Source {
            core: core.clone(),
            demand: demand.clone(),
            cancel,
            done,
        };
        let expected_generation = generation.clone();
        let expected_id = id.clone();
        let observed_process = process.clone();
        let valid = Arc::new(move || {
            if !observed_process.still_matches()? {
                return Ok(false);
            }
            let r = read_record(&path)?;
            Ok(r["deviceId"].as_str() == Some(expected_id.as_str())
                && r["generation"].as_str() == Some(expected_generation.as_str())
                && r["process"]
                    == serde_json::to_value(&observed_process).map_err(|e| e.to_string())?)
        });
        let run_fence = fence.clone();
        let task_core = core.clone();
        let task = tokio::spawn(async move {
            let _slot = slot;
            let result = run_connected(
                connection,
                run_fence,
                task_core.clone(),
                demand,
                cancelled,
                valid,
            )
            .await;
            task_core.lock().unwrap().clear(if result.is_ok() {
                Health::Ended
            } else {
                Health::Failed
            });
            drop(_slot);
            finished.send_replace(true);
            result
        });
        let epochs = [
            ("local", Kind::Local),
            ("a", Kind::Remote),
            ("b", Kind::Remote),
        ]
        .into_iter()
        .map(|(id, kind)| source.join(id, kind).map(|epoch| (id, epoch)))
        .collect::<Result<BTreeMap<_, _>, _>>()
        .unwrap();
        let result: Result<serde_json::Value, String> = async {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut sinks = BTreeMap::new();
            while sinks.len() != 3 {
                for id in ["local", "a", "b"] { if let Some(d) = source.take(id, &fence, epochs[id]) {
                    if !d.frame.sleeping { if id != "local" { image::load_from_memory(&d.frame.bytes).map_err(|e| e.to_string())?; } sinks.insert(id, d.frame.receipt); }
                    source.ack(id, &fence, d.epoch, d.frame.receipt);
                }}
                if Instant::now() > deadline { return Err("Three native memory sinks did not receive ready pixels".into()); }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            source.hide_local(); let before = core.lock().unwrap().receipt;
            let deadline = Instant::now() + HEALTH_INTERVAL + RPC_TIMEOUT + Duration::from_secs(2);
            let mut after = BTreeMap::new();
            while after.len() != 2 {
                for id in ["a", "b"] { if let Some(d) = source.take(id, &fence, epochs[id]) {
                    if d.frame.receipt > before && !d.frame.sleeping { after.insert(id, d.frame.receipt); }
                    source.ack(id, &fence, d.epoch, d.frame.receipt);
                }}
                if Instant::now() > deadline { return Err("Both remote sinks need a fresh receipt after local hide".into()); }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(process.still_matches()?);
            let c = core.lock().unwrap();
            if c.stream_calls != 1 || c.stream_frames == 0 || c.consumers.len() != 2 { return Err("Native fan-out invariants failed".into()); }
            Ok(serde_json::json!({"streamCalls":c.stream_calls,"streamFrames":c.stream_frames,"unaryCalls":c.unary_calls,"sharedEncodes":c.encodes,"threeMemorySinks":sinks,"remoteReceiptsAfterLocalHide":after,"sameGeneration":true,"sameProcess":true,"noStop":true,"productionRendererOrRemoteTransport":false}))
        }.await;
        source.close().await;
        task.await.unwrap().unwrap();
        // No screenshot, runtime record, credential, JWK, or generation key is printed or saved.
        println!("{}", result.unwrap());
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[tokio::test]
    #[ignore = "Uses an explicitly licensed canonical isolated managed AVD; never use ordinary SDK/devices"]
    async fn native_one_stream_three_memory_sinks_hide_keeps_remote_generation() {
        use super::super::{manager::Android, storage};
        use std::{path::PathBuf, sync::atomic::Ordering};
        let trial = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap())
            .canonicalize()
            .unwrap();
        assert!(trial
            .to_string_lossy()
            .starts_with("/private/tmp/lomi-android-stage0-"));
        let consent: serde_json::Value = storage::read(&trial.join("evidence/consent.json"))
            .unwrap()
            .unwrap();
        assert_eq!(consent["accepted"], true);
        let installation: serde_json::Value =
            storage::read(&trial.join("evidence/native-managed-installation.json"))
                .unwrap()
                .unwrap();
        let root = PathBuf::from(installation["root"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        assert!(root.starts_with(&trial));
        let android = Android::default();
        let manager = android.get(root.clone()).unwrap();
        manager.test_adb_port.store(15047, Ordering::Release);
        // Select only a device already configured inside this isolated managed root.
        let id = manager
            .directory
            .lock()
            .unwrap()
            .devices()
            .unwrap()
            .devices
            .first()
            .unwrap()
            .id
            .clone();
        let started = manager.start(&id).await.unwrap();
        let generation = started.generation.clone().unwrap();
        let runtime = manager.runtime(&id, &generation).unwrap();
        let fence = Fence {
            generation: generation.clone(),
            config: 1,
        };
        let source = Source::new(runtime.clone(), fence.clone(), (360, 640)).unwrap();
        let epochs = [
            ("local", Kind::Local),
            ("remote-a", Kind::Remote),
            ("remote-b", Kind::Remote),
        ]
        .into_iter()
        .map(|(id, kind)| source.join(id, kind).map(|epoch| (id, epoch)))
        .collect::<Result<BTreeMap<_, _>, _>>()
        .unwrap();
        let result: Result<serde_json::Value, String> = async {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut receipts = BTreeMap::new();
            while receipts.len() != 3 {
                for id in ["local", "remote-a", "remote-b"] {
                    if let Some(delivery) = source.take(id, &fence, epochs[id]) {
                        if !delivery.frame.sleeping {
                            if id != "local" { image::load_from_memory(&delivery.frame.bytes).map_err(|e| e.to_string())?; }
                            receipts.insert(id, delivery.frame.receipt);
                        }
                        source.ack(id, &fence, delivery.epoch, delivery.frame.receipt);
                    }
                }
                if Instant::now() >= deadline { return Err("No real native frame in each of three sinks".into()); }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            source.hide_local();
            let before = source.core.lock().unwrap().receipt;
            let deadline = Instant::now() + HEALTH_INTERVAL + RPC_TIMEOUT + Duration::from_secs(2);
            let mut after = None;
            while after.is_none() {
                for id in ["remote-a", "remote-b"] {
                    if let Some(delivery) = source.take(id, &fence, epochs[id]) {
                        if delivery.frame.receipt > before { after = Some(delivery.frame.receipt); }
                        source.ack(id, &fence, delivery.epoch, delivery.frame.receipt);
                    }
                }
                if Instant::now() >= deadline { return Err("No fresh remote source receipt after local hide".into()); }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let c = source.core.lock().unwrap();
            if c.stream_calls != 1 || c.stream_frames == 0 || c.consumers.contains_key("local") || c.consumers.len() != 2 || runtime.status().generation != Some(generation.clone()) || runtime.status().phase != Phase::Running { return Err("Source/consumer/generation invariant failed".into()); }
            Ok(serde_json::json!({"source":"real authenticated streamScreenshot plus bounded getScreenshot", "streamCalls":c.stream_calls,"streamFrames":c.stream_frames,"unaryCalls":c.unary_calls,"encodes":c.encodes,"generation":generation,"threeSinkReceipts":receipts,"afterHideReceipt":after,"localHideKeepsRemote":true,"memorySinksOnly":true,"productionTauriChannelOrRemoteTransport":false}))
        }.await;
        source.close().await;
        manager.stop(&id, false).await.unwrap();
        manager.stop_private_adb_fixture().unwrap();
        let report = result.unwrap();
        storage::write(
            &trial.join("evidence/native-remote-source-prototype.json"),
            &report,
        )
        .unwrap();
        println!("{report}");
    }
}
