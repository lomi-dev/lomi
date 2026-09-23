use image::ImageEncoder;
use lomi_control_core::{
    android::AndroidControl,
    artifacts::{ProducerPermit, MAX_IMAGE_BYTES},
    broker::AndroidCapture,
};
use lomi_control_protocol::{
    android::AndroidScreenshotInput, artifact::AndroidImageGeometry, ErrorCode,
};
use std::{
    io::{Cursor, Write},
    sync::Arc,
    time::Instant,
};
use tauri::Manager as _;

pub(crate) fn capture(
    app: &tauri::AppHandle,
    control: Arc<AndroidControl>,
    input: AndroidScreenshotInput,
    deadline: Instant,
) -> Result<AndroidCapture, ErrorCode> {
    control.check_generation(&input.generation)?;
    let _producer = ProducerPermit::acquire()?;
    let window = app.get_window("main").ok_or(ErrorCode::UiNotReady)?;
    let manager = super::commands::backend(&window, &app.state::<super::manager::Android>())
        .map_err(|_| ErrorCode::StorageUnavailable)?;
    let runtime = manager
        .agent_runtime(&control, &input.generation)
        .map_err(|_| ErrorCode::StaleGeneration)?;
    let display = runtime.status().display.ok_or(ErrorCode::TargetBusy)?;
    let edge = u32::from(input.max_edge).min(1414);
    let frame = tauri::async_runtime::block_on(async {
        tokio::time::timeout_at(deadline.into(), async {
            let connection = runtime
                .connection()
                .await
                .map_err(|_| ErrorCode::StaleGeneration)?;
            control.check_generation(&input.generation)?;
            if Instant::now() >= deadline {
                return Err(ErrorCode::DeadlineExceeded);
            }
            let request = connection
                .request(
                    "getScreenshot",
                    crate::android_protocol::ImageFormat {
                        format: 0,
                        width: edge,
                        height: edge,
                        ..Default::default()
                    },
                )
                .map_err(|_| ErrorCode::ControlRevoked)?;
            connection
                .client()
                .max_decoding_message_size(MAX_IMAGE_BYTES + 65536)
                .get_screenshot(request)
                .await
                .map(|r| r.into_inner())
                .map_err(|_| ErrorCode::UnsupportedCapability)
        })
        .await
        .map_err(|_| ErrorCode::DeadlineExceeded)?
    })?;
    control.check_generation(&input.generation)?;
    let format = frame.format.as_ref().ok_or(ErrorCode::OutcomeUnknown)?;
    let rotation = format.rotation.as_ref().map_or(0, |r| r.rotation);
    if !(0..=3).contains(&rotation)
        || format.width == 0
        || format.height == 0
        || format.width > edge
        || format.height > edge
        || u64::from(format.width) * u64::from(format.height) > 2_000_000
        || frame.image.len() > MAX_IMAGE_BYTES
    {
        return Err(ErrorCode::PanelNotRenderable);
    }
    let mut reader =
        image::ImageReader::with_format(Cursor::new(&frame.image), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(edge);
    limits.max_image_height = Some(edge);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let pixels = reader
        .decode()
        .map_err(|_| ErrorCode::OutcomeUnknown)?
        .into_rgba8();
    if pixels.dimensions() != (format.width, format.height) {
        return Err(ErrorCode::OutcomeUnknown);
    }
    let mut output = BoundedPng {
        bytes: Vec::new(),
        limit: input.max_bytes as usize,
    };
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(
            &pixels,
            format.width,
            format.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| ErrorCode::ArtifactTooLarge)?;
    control.check_generation(&input.generation)?;
    if Instant::now() >= deadline {
        return Err(ErrorCode::DeadlineExceeded);
    }
    let hardware = [display.0, display.1];
    let (w, h) = if rotation % 2 == 0 {
        display
    } else {
        (display.1, display.0)
    };
    Ok(AndroidCapture {
        bytes: output.bytes,
        geometry: AndroidImageGeometry {
            captured_at_millis: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| ErrorCode::AppUnavailable)?
                .as_millis()
                .to_string(),
            hardware_display: hardware,
            rotation: rotation as u8,
            pixel_width: format.width,
            pixel_height: format.height,
            capture_scale_x: f64::from(format.width) / f64::from(w),
            capture_scale_y: f64::from(format.height) / f64::from(h),
            image_to_hardware: AndroidImageGeometry::transform(
                hardware,
                format.width,
                format.height,
                rotation as u8,
            ),
            coordinate_space: "image_pixels".into(),
            crop: "full_display".into(),
        },
    })
}
struct BoundedPng {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedPng {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("Android PNG budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
