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
