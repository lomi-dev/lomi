//! Snapshot only a registered child view. Native dimensions and producer permits
//! bound allocation before requesting pixels; PNG encoding runs off AppKit.
use image::ImageEncoder;
use lomi_control_core::{
    artifacts::ProducerPermit, broker::BrowserCapture, browser::BrowserControl,
};
use lomi_control_protocol::{
    control::{BrowserImageGeometry, BrowserScreenshotInput, ImageGeometry},
    ErrorCode,
};
use std::{
    io::{self, Cursor, Write},
    sync::{mpsc::sync_channel, Arc, Mutex},
    time::Instant,
};
use tauri::{AppHandle, Manager};

pub fn capture(
    app: &AppHandle,
    control: Arc<BrowserControl>,
    input: BrowserScreenshotInput,
    deadline: Instant,
) -> Result<BrowserCapture, ErrorCode> {
    control.require_renderable()?;
    let producer = ProducerPermit::acquire()?;
    let geometry =
        super::agent_dom::geometry(app, control.clone(), &input.navigation_id, deadline)?;
    if ![
        geometry.width,
        geometry.height,
        geometry.device_scale_factor,
    ]
    .into_iter()
    .all(|v| v.is_finite() && v > 0.)
    {
        return Err(ErrorCode::PanelNotRenderable);
    }
    if geometry.url != control.document_url()? {
        control.document_changed(&geometry.url);
        return Err(ErrorCode::StaleSnapshot);
    }
    let label = super::label(&control.panel_id).map_err(|_| ErrorCode::TargetNotFound)?;
    {
        let state = app.state::<super::Browsers>();
        let pages = state.pages.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        let page = pages
            .get(&control.panel_id)
            .ok_or(ErrorCode::TargetNotFound)?;
        if page.bounds.is_none() {
            return Err(ErrorCode::PanelNotRenderable);
        }
        if page
            .control
            .as_ref()
            .is_none_or(|c| !Arc::ptr_eq(c, &control))
        {
            return Err(ErrorCode::StaleGeneration);
        }
    }
    let view = app.get_webview(&label).ok_or(ErrorCode::TargetNotFound)?;
    let guard = control.begin_native_dom()?;
    let (send, receive) = sync_channel(1);
    let expected_url = geometry.url.clone();
    let navigation = input.navigation_id.clone();
    let verify_control = control.clone();
    view.with_webview(move |platform| {
        use objc2::{rc::Weak, MainThreadMarker};
        use objc2_app_kit::{NSImage, NSView};
        use objc2_foundation::{NSError, NSNumber};
        use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
        let native = unsafe { &*platform.inner().cast::<WKWebView>() };
        let nsview: &NSView = native;
        let setup = (|| {
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            control.check_document(&navigation)?;
            control.require_renderable()?;
            let window = nsview.window().ok_or(ErrorCode::PanelNotRenderable)?;
            if nsview.isHiddenOrHasHiddenAncestor() || window.isMiniaturized() {
                return Err(ErrorCode::PanelNotRenderable);
            }
            let rect = nsview.bounds();
            let scale = window.backingScaleFactor();
            if ![rect.size.width, rect.size.height, scale]
                .into_iter()
                .all(|v| v.is_finite() && v > 0.)
            {
                return Err(ErrorCode::PanelNotRenderable);
            }
            let width = f64::from(input.max_width)
                .min(rect.size.width * scale)
                .min((2_000_000. * rect.size.width / rect.size.height).sqrt())
                .floor();
            if width < 1. {
                return Err(ErrorCode::PanelNotRenderable);
            }
            let config = unsafe { WKSnapshotConfiguration::new(MainThreadMarker::new().unwrap()) };
            unsafe {
                config.setRect(rect);
                config.setSnapshotWidth(Some(&NSNumber::new_f64(width / scale)));
                config.setAfterScreenUpdates(true);
            }
            Ok((config, rect, unsafe { native.pageZoom() }))
        })();
        let (config, rect, zoom) = match setup {
            Ok(v) => v,
            Err(e) => {
                let _ = send.send(Err(e));
                return;
            }
        };
        let weak = Weak::new(native);
        let permits = Mutex::new(Some((guard, producer)));
        let send = Mutex::new(Some(send));
        let callback = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let result = (|| {
                control.check_document(&navigation)?;
                control.require_renderable()?;
                if Instant::now() >= deadline {
                    return Err(ErrorCode::DeadlineExceeded);
                }
                let native = weak.load().ok_or(ErrorCode::TargetNotFound)?;
                let nsview: &NSView = &native;
                let url = unsafe { native.URL() }
                    .and_then(|u| u.absoluteString())
                    .ok_or(ErrorCode::StaleSnapshot)?
                    .to_string();
                if url != expected_url
                    || nsview.bounds() != rect
                    || (unsafe { native.pageZoom() } - zoom).abs() > 1e-9
                {
                    return Err(ErrorCode::StaleSnapshot);
                }
                if nsview.isHiddenOrHasHiddenAncestor() || !error.is_null() || image.is_null() {
                    return Err(ErrorCode::PanelNotRenderable);
                }
                let image = unsafe { &*image };
                let tiff = image
                    .TIFFRepresentation()
                    .ok_or(ErrorCode::OutcomeUnknown)?;
                if tiff.len() > 16 * 1024 * 1024 {
                    return Err(ErrorCode::ArtifactTooLarge);
                }
                let captured_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| ErrorCode::AppUnavailable)?
                    .as_millis()
                    .to_string();
                Ok((
                    unsafe { tiff.as_bytes_unchecked() }.to_vec(),
                    zoom,
                    captured_at,
                ))
            })();
            let permits = permits.lock().ok().and_then(|mut p| p.take());
            let result = result.and_then(|(bytes, zoom, captured_at)| {
                let (guard, producer) = permits.ok_or(ErrorCode::OutcomeUnknown)?;
                drop(guard);
                Ok((bytes, zoom, captured_at, producer))
            });
            if let Some(send) = send.lock().ok().and_then(|mut s| s.take()) {
                let _ = send.send(result);
            }
        });
        unsafe {
            native.takeSnapshotWithConfiguration_completionHandler(Some(&config), &callback);
        }
    })
    .map_err(|_| ErrorCode::AppUnavailable)?;
    let (tiff, zoom, captured_at, _producer) = receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| ErrorCode::DeadlineExceeded)??;
    let mut reader = image::ImageReader::with_format(Cursor::new(tiff), image::ImageFormat::Tiff);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| ErrorCode::ArtifactTooLarge)?;
    if decoded.width() > u32::from(input.max_width)
        || u64::from(decoded.width()) * u64::from(decoded.height()) > 2_000_000
    {
        return Err(ErrorCode::ArtifactTooLarge);
    }
    let pixels = decoded.into_rgba8();
    let (width, height) = pixels.dimensions();
    let mut output = BoundedPng {
        bytes: Vec::with_capacity(input.max_bytes as usize),
        limit: input.max_bytes as usize,
        exceeded: false,
    };
    let encoded = image::codecs::png::PngEncoder::new_with_quality(
        &mut output,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        pixels.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgba8,
    );
    if encoded.is_err() {
        return Err(if output.exceeded {
            ErrorCode::ArtifactTooLarge
        } else {
            ErrorCode::OutcomeUnknown
        });
    }
    verify_control.check_document(&input.navigation_id)?;
    Ok(BrowserCapture {
        bytes: output.bytes,
        url: geometry.url,
        geometry: ImageGeometry::Browser(BrowserImageGeometry {
            captured_at_millis: captured_at,
            css_width: geometry.width,
            css_height: geometry.height,
            device_scale_factor: geometry.device_scale_factor,
            pixel_width: width,
            pixel_height: height,
            capture_scale_x: f64::from(width) / geometry.width,
            capture_scale_y: f64::from(height) / geometry.height,
            page_zoom: zoom,
            scroll_x: geometry.scroll_x,
            scroll_y: geometry.scroll_y,
            crop: "viewport".into(),
        }),
    })
}
struct BoundedPng {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for BoundedPng {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("Image byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
