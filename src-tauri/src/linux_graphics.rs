//! Process-local Linux graphics defaults, applied before WebKit starts.
use std::{ffi::OsStr, fs, path::Path};

/// Run before GUI threads or graphics contexts are created. Existing user values,
/// including an empty value, always take precedence over this process default.
pub fn configure() {
    if should_default_nvidia_wayland(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("GDK_BACKEND").as_deref(),
        std::env::var_os("WEBKIT_FORCE_DMABUF_RENDERER").as_deref(),
        Path::new("/sys/module/nvidia"),
    ) {
        // Ubuntu's disable-nvidia-dmabuf.patch otherwise rejects NVIDIA before
        // enabling accelerated compositing. Retain its explicit compatibility
        // opt-in independently of page-rendering and explicit-sync policies.
        // https://archive.ubuntu.com/ubuntu/pool/main/w/webkit2gtk/webkit2gtk_2.52.6-0ubuntu0.26.04.1.debian.tar.xz
        std::env::set_var("WEBKIT_FORCE_DMABUF_RENDERER", "1");
    }
    for name in [
        "WEBKIT_SKIA_ENABLE_CPU_RENDERING",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
    ] {
        if should_default_nvidia_wayland(
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            std::env::var_os("GDK_BACKEND").as_deref(),
            std::env::var_os(name).as_deref(),
            Path::new("/sys/module/nvidia"),
        ) {
            // Process-wide WebKit page defaults, not an Android-only policy.
            // An explicit caller value for either setting always wins.
            std::env::set_var(name, "1");
        }
    }
    if should_disable_kopper(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("GDK_BACKEND").as_deref(),
        std::env::var_os("LIBGL_KOPPER_DISABLE").as_deref(),
        Path::new("/sys/class/drm"),
    ) {
        // Mesa's Kopper switch selects the DRI presentation path instead of Zink's
        // Vulkan WSI swapchain. It does not disable WebKit DMA-BUF or GPU rendering.
        // https://docs.mesa3d.org/envvars.html#libgl-kopper-disable
        std::env::set_var("LIBGL_KOPPER_DISABLE", "true");
    }
    if should_force_nouveau_x11_shm(
        std::env::var_os("DISPLAY").as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("GDK_BACKEND").as_deref(),
        std::env::var_os("WEBKIT_DMABUF_RENDERER_FORCE_SHM").as_deref(),
        std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").as_deref(),
        Path::new("/sys/class/drm"),
    ) {
        // WebKitGTK 2.54 can use shared-memory buffers without GBM while keeping
        // GPU compositing. This switch is unavailable in older WebKitGTK releases.
        std::env::set_var("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1");
    }
}

fn should_force_nouveau_x11_shm(
    display: Option<&OsStr>,
    wayland_display: Option<&OsStr>,
    gdk_backend: Option<&OsStr>,
    existing: Option<&OsStr>,
    disable_dmabuf: Option<&OsStr>,
    drm: &Path,
) -> bool {
    existing.is_none()
        && disable_dmabuf.is_none()
        && is_x11_candidate(display, wayland_display, gdk_backend)
        && has_nouveau_render_device(drm)
}

fn is_x11_candidate(
    display: Option<&OsStr>,
    wayland_display: Option<&OsStr>,
    gdk_backend: Option<&OsStr>,
) -> bool {
    if !display.is_some_and(|value| !value.is_empty()) {
        return false;
    }
    let has_wayland_display = wayland_display.is_some_and(|value| !value.is_empty());
    match gdk_backend {
        Some(backends) => backends.to_str().is_some_and(|backends| {
            // In a Wayland session, only an explicit first-choice X11 backend
            // qualifies. A fallback or wildcard may still select passing Wayland.
            if has_wayland_display {
                backends
                    .split(',')
                    .next()
                    .is_some_and(|backend| backend == "x11")
            } else {
                backends
                    .split(',')
                    .any(|backend| matches!(backend, "x11" | "*"))
            }
        }),
        None => !has_wayland_display,
    }
}

fn should_disable_kopper(
    wayland_display: Option<&OsStr>,
    gdk_backend: Option<&OsStr>,
    existing: Option<&OsStr>,
    drm: &Path,
) -> bool {
    existing.is_none()
        && is_wayland_candidate(wayland_display, gdk_backend)
        && has_nouveau_render_device(drm)
}

fn should_default_nvidia_wayland(
    wayland_display: Option<&OsStr>,
    gdk_backend: Option<&OsStr>,
    existing: Option<&OsStr>,
    nvidia_module: &Path,
) -> bool {
    existing.is_none()
        && is_wayland_candidate(wayland_display, gdk_backend)
        && nvidia_module.exists()
}

fn is_wayland_candidate(wayland_display: Option<&OsStr>, gdk_backend: Option<&OsStr>) -> bool {
    if !wayland_display.is_some_and(|value| !value.is_empty()) {
        return false;
    }
    // Respect an explicit backend selection that cannot choose Wayland, including
    // X11 in a Wayland desktop session. GDK accepts a comma-separated fallback list.
    if let Some(backends) = gdk_backend {
        let Some(backends) = backends.to_str() else {
            return false;
        };
        if !backends
            .split(',')
            .any(|backend| matches!(backend.trim(), "wayland" | "*"))
        {
            return false;
        }
    }
    true
}

fn has_nouveau_render_device(drm: &Path) -> bool {
    let Ok(entries) = fs::read_dir(drm) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(suffix) = name.to_str().and_then(|name| name.strip_prefix("renderD")) else {
            return false;
        };
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        // A resolved device/driver binding qualifies the GPU. A loaded Nouveau
        // module or an unbound/card-only device alone does not qualify it.
        fs::canonicalize(entry.path().join("device/driver"))
            .ok()
            .is_some_and(|driver| driver.file_name() == Some(OsStr::new("nouveau")))
    })
}
