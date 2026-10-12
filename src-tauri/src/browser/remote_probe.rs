//! Isolated ordinary-page qualification. This module has no remote admission or IPC.
use std::{
    collections::HashSet,
    io::{self, Cursor, Write},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use tauri::Manager;

#[derive(Default)]
struct Slots(Mutex<HashSet<String>>);
struct Permit {
    slots: Arc<Slots>,
    label: String,
}
impl Slots {
    fn acquire(self: &Arc<Self>, label: &str) -> Result<Permit, String> {
        let mut slots = self.0.lock().map_err(|_| "Slot lock failed")?;
        if slots.len() >= 2 || !slots.insert(label.into()) {
            return Err("Capture busy".into());
        }
        Ok(Permit {
            slots: self.clone(),
            label: label.into(),
        })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.slots.0.lock() {
            slots.remove(&self.label);
        }
    }
}
#[derive(Clone, PartialEq)]
struct Fence {
    incarnation: String,
    content: u64,
    geometry: u64,
    url: String,
    bounds: Option<super::Bounds>,
}
fn fence(app: &tauri::AppHandle, id: &str) -> Result<Fence, String> {
    let state = app.state::<super::Browsers>();
    let pages = state.pages.lock().map_err(|_| "Page lock failed")?;
    let page = pages.get(id).ok_or("Closed page")?;
    if page.control.is_some() {
        return Err("Probe requires ordinary page".into());
    }
    Ok(Fence {
        incarnation: page.incarnation.clone(),
        content: page.content_epoch,
        geometry: page.geometry_epoch,
        url: page.url.clone(),
        bounds: page.bounds,
    })
}
fn verify(app: &tauri::AppHandle, id: &str, expected: &Fence) -> Result<(), String> {
    if fence(app, id)? != *expected {
        return Err("Stale page identity/navigation/geometry".into());
    }
    Ok(())
}
pub(crate) struct Frame {
    pub jpeg: Vec<u8>,
    pub metrics: serde_json::Value,
    pub center: [u8; 3],
    pub clock_pixel: [u8; 3],
    pub content_pixels: serde_json::Value,
}

/// Call only from the fixture's blocking worker. Timeout never releases a native slot.
pub(crate) fn capture(
    app: &tauri::AppHandle,
    id: &str,
    deadline: Instant,
) -> Result<Frame, String> {
    static SLOTS: OnceLock<Arc<Slots>> = OnceLock::new();
    let label = super::label(id)?;
    let permit = SLOTS
        .get_or_init(|| Arc::new(Slots::default()))
        .acquire(&label)?;
    let expected = fence(app, id)?;
    let view = app.get_webview(&label).ok_or("Missing view")?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let callback_app = app.clone();
    let callback_id = id.to_owned();
    let callback_fence = expected.clone();
    let began = Instant::now();
    view.with_webview(move |platform| {
        use objc2::{rc::Weak, MainThreadMarker};
        use objc2_app_kit::{NSImage, NSView};
        use objc2_foundation::{NSError, NSNumber};
        use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
        if Instant::now() >= deadline || verify(&callback_app, &callback_id, &callback_fence).is_err() {
            let _ = send.send(Err("Stale or expired capture setup".to_string()));
            return;
        }
        let native = unsafe { &*platform.inner().cast::<WKWebView>() };
        let nsview: &NSView = native;
        let rect = nsview.bounds();
        let scale = nsview.window().map(|w| w.backingScaleFactor()).unwrap_or(1.);
        let zoom = unsafe { native.pageZoom() };
        if ![rect.size.width, rect.size.height, scale, zoom].into_iter().all(|n| n.is_finite() && n > 0.) {
            let _ = send.send(Err("Invalid native geometry".into())); return;
        }
        let aspect = rect.size.width / rect.size.height;
        let width = (rect.size.width * scale).min(1280.).min(1280. * aspect).min((921600. * aspect).sqrt()).floor();
        if width < 1. { let _ = send.send(Err("Empty snapshot geometry".into())); return; }
        let config = unsafe { WKSnapshotConfiguration::new(MainThreadMarker::new().unwrap()) };
        unsafe { config.setRect(rect); config.setSnapshotWidth(Some(&NSNumber::new_f64(width / scale))); config.setAfterScreenUpdates(true); }
        let visibility = serde_json::json!({"hidden":nsview.isHiddenOrHasHiddenAncestor(),"minimized":nsview.window().is_some_and(|w| w.isMiniaturized()),"windowVisible":nsview.window().is_some_and(|w| w.isVisible()),"occlusionVisible":nsview.window().is_some_and(|w| w.occlusionState().contains(objc2_app_kit::NSWindowOcclusionState::Visible)),"nativeWindowNumber":nsview.window().map(|w|w.windowNumber()),"nativeWindowKey":nsview.window().is_some_and(|w|w.isKeyWindow()),"nativeWindowMain":nsview.window().is_some_and(|w|w.isMainWindow()),"ignoresMouseEvents":nsview.window().is_some_and(|w|w.ignoresMouseEvents()),"registeredVisible":callback_fence.bounds.is_some(),"cssWidth":rect.size.width,"cssHeight":rect.size.height,"deviceScale":scale,"zoom":zoom});
        let weak = Weak::new(native);
        // The block owns admission through actual settlement, even if the receiver times out.
        let pending = Mutex::new(Some((send, permit)));
        let callback = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let result: Result<_, String> = (|| {
                verify(&callback_app, &callback_id, &callback_fence)?;
                if Instant::now() >= deadline { return Err("Capture deadline exceeded".into()); }
                let native = weak.load().ok_or("Closed native view")?;
                let nsview: &NSView = &native;
                let url = unsafe { native.URL() }.and_then(|u| u.absoluteString()).map(|s| s.to_string());
                if url.as_deref() != Some(callback_fence.url.as_str()) || nsview.bounds() != rect || (unsafe { native.pageZoom() } - zoom).abs() > 1e-9 { return Err("Stale native navigation/geometry".into()); }
                if !error.is_null() || image.is_null() { return Err("Native snapshot failed".into()); }
                let tiff = unsafe { &*image }.TIFFRepresentation().ok_or("Missing TIFF")?;
                if tiff.len() > 8 * 1024 * 1024 { return Err("TIFF exceeds byte bound".into()); }
                Ok((unsafe { tiff.as_bytes_unchecked() }.to_vec(), visibility.clone(), began.elapsed()))
            })();
            if let Some((send, permit)) = pending.lock().ok().and_then(|mut p| p.take()) {
                // Keep the same bounded slot through decode and encode as well.
                let _ = send.send(result.map(|v| (v, permit)));
            }
        });
        unsafe { native.takeSnapshotWithConfiguration_completionHandler(Some(&config), &callback); }
    }).map_err(|e| e.to_string())?;
    let ((tiff, visibility, capture_time), _permit) = receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Capture deadline exceeded")??;
    let encode_began = Instant::now();
    let tiff_bytes = tiff.len();
    let mut reader = image::ImageReader::with_format(Cursor::new(tiff), image::ImageFormat::Tiff);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1280);
    limits.max_image_height = Some(1280);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?;
    if u64::from(decoded.width()) * u64::from(decoded.height()) > 921600 {
        return Err("Pixel bound exceeded".into());
    }
    // Composite alpha on the ordinary page's white background before JPEG encoding.
    let mut rgba = decoded.into_rgba8();
    for pixel in rgba.pixels_mut() {
        let a = u32::from(pixel[3]);
        for c in 0..3 {
            pixel[c] = ((u32::from(pixel[c]) * a + 255 * (255 - a)) / 255) as u8;
        }
        pixel[3] = 255;
    }
    let rgb = image::DynamicImage::ImageRgba8(rgba).into_rgb8();
    let center = rgb.get_pixel(rgb.width() / 2, rgb.height() / 2).0;
    let clock_offset =
        (20. * rgb.height() as f64 / visibility["cssHeight"].as_f64().unwrap_or(1.)).round() as u32;
    let clock_y = rgb
        .height()
        .saturating_sub(clock_offset.max(1))
        .min(rgb.height() - 1);
    let clock_pixel = rgb.get_pixel(rgb.width() / 2, clock_y).0;
    let sample = |x: f64, y: f64| {
        rgb.get_pixel(
            ((x * rgb.width() as f64 / visibility["cssWidth"].as_f64().unwrap_or(1.)).round()
                as u32)
                .min(rgb.width() - 1),
            ((y * rgb.height() as f64 / visibility["cssHeight"].as_f64().unwrap_or(1.)).round()
                as u32)
                .min(rgb.height() - 1),
        )
        .0
    };
    let content_pixels = serde_json::json!({"canvas2d":sample(100.,190.),"webgl":sample(300.,190.),"rafDom":sample(500.,190.)});
    let mut output = Bounded { bytes: Vec::new() };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 70)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;
    let (check_send, check_receive) = std::sync::mpsc::sync_channel(1);
    let final_url = expected.url.clone();
    let final_geometry = visibility.clone();
    view.with_webview(move |platform| {
        use objc2_app_kit::NSView;
        use objc2_web_kit::WKWebView;
        let native = unsafe { &*platform.inner().cast::<WKWebView>() };
        let nsview: &NSView = native;
        let rect = nsview.bounds();
        let current_url = unsafe { native.URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string());
        let valid = current_url.as_deref() == Some(final_url.as_str())
            && Some(rect.size.width) == final_geometry["cssWidth"].as_f64()
            && Some(rect.size.height) == final_geometry["cssHeight"].as_f64()
            && Some(unsafe { native.pageZoom() }) == final_geometry["zoom"].as_f64();
        let _ = check_send.send(valid);
    })
    .map_err(|e| e.to_string())?;
    if !check_receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "Publication check deadline exceeded")?
    {
        return Err("Stale native geometry before publication".into());
    }
    verify(app, id, &expected)?;
    if Instant::now() >= deadline {
        return Err("Encode deadline exceeded".into());
    }
    let metrics = serde_json::json!({"visibility":visibility,"captureMillis":capture_time.as_secs_f64()*1000.,"decodeEncodeMillis":encode_began.elapsed().as_secs_f64()*1000.,"tiffBytes":tiff_bytes,"jpegBytes":output.bytes.len(),"pixelWidth":rgb.width(),"pixelHeight":rgb.height(),"contentEpoch":expected.content,"geometryEpoch":expected.geometry,"incarnation":expected.incarnation});
    Ok(Frame {
        jpeg: output.bytes,
        metrics,
        center,
        clock_pixel,
        content_pixels,
    })
}
struct Bounded {
    bytes: Vec<u8>,
}
impl Write for Bounded {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.len() > (512 * 1024usize).saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("JPEG exceeds byte bound"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeout_and_restart_do_not_release_native_admission() {
        let slots = Arc::new(Slots::default());
        let first = slots.acquire("same").unwrap();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let settle = move || {
            let _ = sender.send(());
            drop(first);
        };
        assert!(receiver.recv_timeout(std::time::Duration::ZERO).is_err());
        drop(receiver);
        for _ in 0..100 {
            assert!(slots.acquire("same").is_err());
        }
        let second = slots.acquire("other").unwrap();
        assert!(slots.acquire("third").is_err());
        settle();
        assert!(slots.acquire("same").is_ok());
        drop(second);
    }
    #[test]
    fn stale_identity_navigation_and_geometry_are_distinct() {
        let original = Fence {
            incarnation: "one".into(),
            content: 1,
            geometry: 1,
            url: "http://localhost/".into(),
            bounds: None,
        };
        let mut changed = original.clone();
        changed.incarnation = "two".into();
        assert!(changed != original);
        changed = original.clone();
        changed.content += 1;
        assert!(changed != original);
        changed = original.clone();
        changed.geometry += 2; // Resize away and back must still fence the old frame.
        assert!(changed != original);
        changed = original.clone();
        changed.bounds = Some(super::super::Bounds {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        });
        assert!(changed != original);
    }
    #[test]
    fn encoded_byte_limit_is_hard() {
        let mut output = Bounded {
            bytes: vec![0; 512 * 1024 - 1],
        };
        assert!(output.write(&[1, 2]).is_err());
        assert_eq!(output.bytes.len(), 512 * 1024 - 1);
    }
}
