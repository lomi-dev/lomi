use std::ffi::OsStr;

pub(crate) fn select(platform: &str, disable_compositing: Option<&OsStr>) -> &'static str {
    // WebKit treats any present value except exactly "0" as disabling compositing.
    // Avoid WebGL canvas readback through CPU image buffers in that mode.
    if platform == "linux" && disable_compositing.is_some_and(|value| value != OsStr::new("0")) {
        "dom"
    } else {
        "webgl"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_follows_linux_compositing_semantics() {
        for platform in ["linux", "macos", "windows"] {
            for (value, linux_renderer) in [
                (None, "webgl"),
                (Some(OsStr::new("0")), "webgl"),
                (Some(OsStr::new("1")), "dom"),
                (Some(OsStr::new("")), "dom"),
                (Some(OsStr::new("false")), "dom"),
            ] {
                assert_eq!(
                    select(platform, value),
                    if platform == "linux" {
                        linux_renderer
                    } else {
                        "webgl"
                    },
                    "{platform}: {value:?}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_value_disables_compositing_only_on_linux() {
        use std::os::unix::ffi::OsStrExt;
        let value = OsStr::from_bytes(&[0xff]);
        assert_eq!(select("linux", Some(value)), "dom");
        assert_eq!(select("macos", Some(value)), "webgl");
    }
}
