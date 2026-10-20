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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct DrmFixture(PathBuf);
    impl DrmFixture {
        fn new() -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "lomi-linux-graphics-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn bind(&self, node: &str, driver: &str) {
            let target = self.0.join("drivers").join(driver);
            fs::create_dir_all(&target).unwrap();
            let device = self.0.join(node).join("device");
            fs::create_dir_all(&device).unwrap();
            std::os::unix::fs::symlink(target, device.join("driver")).unwrap();
        }
        fn policy(
            &self,
            display: Option<&str>,
            backend: Option<&str>,
            existing: Option<&str>,
        ) -> bool {
            should_disable_kopper(
                display.map(OsStr::new),
                backend.map(OsStr::new),
                existing.map(OsStr::new),
                &self.0,
            )
        }
        fn x11_policy(
            &self,
            display: Option<&str>,
            wayland_display: Option<&str>,
            backend: Option<&str>,
            existing: Option<&str>,
        ) -> bool {
            should_force_nouveau_x11_shm(
                display.map(OsStr::new),
                wayland_display.map(OsStr::new),
                backend.map(OsStr::new),
                existing.map(OsStr::new),
                None,
                &self.0,
            )
        }
    }
    impl Drop for DrmFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn requires_wayland_candidate_and_bound_nouveau_render_device() {
        let drm = DrmFixture::new();
        assert!(!drm.policy(Some("wayland-0"), None, None));
        drm.bind("card0", "nouveau");
        drm.bind("renderDnot-a-node", "nouveau");
        assert!(!drm.policy(Some("wayland-0"), None, None));
        drm.bind("renderD128", "nouveau");
        assert!(drm.policy(Some("wayland-0"), None, None));
        assert!(drm.policy(Some("wayland-0"), Some("wayland,x11"), None));
        assert!(drm.policy(Some("wayland-0"), Some("*"), None));
        for display in [None, Some("")] {
            assert!(!drm.policy(display, None, None));
        }
        for backend in ["x11", "broadway", "", "x11,broadway"] {
            assert!(!drm.policy(Some("wayland-0"), Some(backend), None));
        }
    }

    #[test]
    fn preserves_every_existing_kopper_value() {
        let drm = DrmFixture::new();
        drm.bind("renderD128", "nouveau");
        for existing in ["", "false", "0", "true", "1", "custom"] {
            assert!(!drm.policy(Some("wayland-0"), None, Some(existing)));
        }
    }

    #[test]
    fn does_not_apply_nouveau_policy_to_proprietary_or_other_drivers() {
        let drm = DrmFixture::new();
        drm.bind("renderD128", "nvidia");
        drm.bind("renderD129", "i915");
        assert!(!drm.policy(Some("wayland-0"), None, None));
        // A broken binding must not qualify merely because its target is named nouveau.
        let device = drm.0.join("renderD130/device");
        fs::create_dir_all(&device).unwrap();
        std::os::unix::fs::symlink(drm.0.join("missing/nouveau"), device.join("driver")).unwrap();
        assert!(!drm.policy(Some("wayland-0"), None, None));
        assert!(!should_disable_kopper(
            Some(OsStr::new("wayland-0")),
            None,
            None,
            &drm.0.join("absent")
        ));
    }

    #[test]
    fn nvidia_defaults_require_module_and_wayland_candidate() {
        let fixture = DrmFixture::new();
        let module = fixture.0.join("module/nvidia");
        let policy = |display: Option<&str>, backend: Option<&str>| {
            should_default_nvidia_wayland(
                display.map(OsStr::new),
                backend.map(OsStr::new),
                None,
                &module,
            )
        };
        assert!(!policy(Some("wayland-0"), None));
        fixture.bind("renderD128", "nouveau");
        assert!(!policy(Some("wayland-0"), None));
        fs::create_dir_all(&module).unwrap();
        assert!(policy(Some("wayland-0"), None));
        for backend in ["wayland", "x11,wayland", " * "] {
            assert!(policy(Some("wayland-0"), Some(backend)));
        }
        for display in [None, Some("")] {
            assert!(!policy(display, None));
        }
        for backend in ["x11", "broadway", "", "x11,broadway"] {
            assert!(!policy(Some("wayland-0"), Some(backend)));
        }
    }

    #[test]
    fn nvidia_defaults_preserve_every_existing_os_value() {
        use std::os::unix::ffi::OsStrExt;

        let fixture = DrmFixture::new();
        let module = fixture.0.join("module/nvidia");
        fs::create_dir_all(&module).unwrap();
        for existing in ["", "0", "1", "false", "custom"].map(OsStr::new) {
            assert!(!should_default_nvidia_wayland(
                Some(OsStr::new("wayland-0")),
                None,
                Some(existing),
                &module
            ));
        }
        let non_unicode = OsStr::from_bytes(&[0xff]);
        assert!(!should_default_nvidia_wayland(
            Some(OsStr::new("wayland-0")),
            None,
            Some(non_unicode),
            &module
        ));
        assert!(!should_default_nvidia_wayland(
            Some(OsStr::new("wayland-0")),
            Some(non_unicode),
            None,
            &module
        ));
    }

    #[test]
    fn nvidia_defaults_preserve_settings_independently() {
        use std::os::unix::ffi::OsStrExt;

        let fixture = DrmFixture::new();
        let module = fixture.0.join("module/nvidia");
        fs::create_dir_all(&module).unwrap();
        let non_unicode = OsStr::from_bytes(&[0xff]);
        for existing in [
            Some(OsStr::new("")),
            Some(OsStr::new("0")),
            Some(non_unicode),
        ] {
            // A user override for one default does not suppress a different,
            // unset default; none of these pure checks mutates process env.
            let decisions = [existing, None, existing].map(|value| {
                should_default_nvidia_wayland(Some(OsStr::new("wayland-0")), None, value, &module)
            });
            assert_eq!(decisions, [false, true, false]);
        }
    }

    #[test]
    fn nouveau_shm_requires_x11_candidate() {
        let drm = DrmFixture::new();
        drm.bind("renderD128", "nouveau");
        assert!(drm.x11_policy(Some(":0"), None, None, None));
        assert!(drm.x11_policy(Some(":0"), Some(""), None, None));
        assert!(!drm.x11_policy(Some(":0"), Some("wayland-0"), None, None));
        for backend in ["x11", "wayland,x11", "x11,broadway", "broadway,x11", "*"] {
            assert!(drm.x11_policy(Some(":0"), None, Some(backend), None));
        }
        for backend in ["x11", "x11,wayland", "x11,*", "x11,broadway"] {
            assert!(drm.x11_policy(Some(":0"), Some("wayland-0"), Some(backend), None));
        }
        for backend in [
            "wayland,x11",
            "*",
            "*,x11",
            "broadway,x11",
            " wayland, x11 ",
            " x11 , *",
            "x11 ,wayland",
            " x11,*",
        ] {
            assert!(!drm.x11_policy(Some(":0"), Some("wayland-0"), Some(backend), None));
        }
        for backend in [
            "",
            "wayland",
            "broadway",
            "wayland,broadway",
            " wayland, x11 ",
            " x11 , *",
            "x11 ,wayland",
        ] {
            assert!(!drm.x11_policy(Some(":0"), None, Some(backend), None));
        }
        for display in [None, Some("")] {
            assert!(!drm.x11_policy(display, None, None, None));
            assert!(!drm.x11_policy(display, Some("wayland-0"), Some("x11"), None));
        }
    }

    #[test]
    fn nouveau_x11_shm_requires_resolved_render_driver_binding() {
        let drm = DrmFixture::new();
        assert!(!drm.x11_policy(Some(":0"), None, None, None));
        drm.bind("renderD128", "nvidia");
        drm.bind("renderD129", "i915");
        drm.bind("card0", "nouveau");
        drm.bind("renderD", "nouveau");
        drm.bind("renderDnot-a-node", "nouveau");
        let device = drm.0.join("renderD130/device");
        fs::create_dir_all(&device).unwrap();
        std::os::unix::fs::symlink(drm.0.join("missing/nouveau"), device.join("driver")).unwrap();
        assert!(!drm.x11_policy(Some(":0"), None, None, None));
        assert!(!should_force_nouveau_x11_shm(
            Some(OsStr::new(":0")),
            None,
            None,
            None,
            None,
            &drm.0.join("absent"),
        ));
        drm.bind("renderD131", "nouveau");
        assert!(drm.x11_policy(Some(":0"), None, None, None));
    }

    #[test]
    fn nouveau_x11_shm_preserves_os_overrides_and_backend_selection() {
        use std::os::unix::ffi::OsStrExt;

        let drm = DrmFixture::new();
        drm.bind("renderD128", "nouveau");
        for existing in ["", "0", "1", "false", "custom"].map(OsStr::new) {
            assert!(!should_force_nouveau_x11_shm(
                Some(OsStr::new(":0")),
                None,
                None,
                Some(existing),
                None,
                &drm.0,
            ));
        }
        let non_unicode = OsStr::from_bytes(&[0xff]);
        assert!(!should_force_nouveau_x11_shm(
            Some(OsStr::new(":0")),
            None,
            None,
            Some(non_unicode),
            None,
            &drm.0,
        ));
        assert!(!should_force_nouveau_x11_shm(
            Some(OsStr::new(":0")),
            None,
            Some(non_unicode),
            None,
            None,
            &drm.0,
        ));
        assert!(!should_force_nouveau_x11_shm(
            Some(OsStr::new(":0")),
            Some(non_unicode),
            None,
            None,
            None,
            &drm.0,
        ));
    }

    #[test]
    fn nouveau_x11_shm_respects_any_explicit_dmabuf_disable_setting() {
        use std::os::unix::ffi::OsStrExt;

        let drm = DrmFixture::new();
        drm.bind("renderD128", "nouveau");
        assert!(drm.x11_policy(Some(":0"), None, None, None));
        for disable_dmabuf in [
            OsStr::new(""),
            OsStr::new("0"),
            OsStr::new("1"),
            OsStr::new("false"),
            OsStr::new("custom"),
            OsStr::from_bytes(&[0xff]),
        ] {
            assert!(!should_force_nouveau_x11_shm(
                Some(OsStr::new(":0")),
                None,
                None,
                None,
                Some(disable_dmabuf),
                &drm.0,
            ));
        }
    }
}
