//! The OS clock preference. WebViews format times from the locale alone and
//! ignore the user's 12/24-hour override, so it is read natively.

/// `Some(true)` for a 12-hour clock, `None` when the platform has no setting.
#[tauri::command]
pub fn system_hour12(window: tauri::Window) -> Result<Option<bool>, String> {
    crate::files::main_window(&window)?;
    Ok(hour12())
}

#[cfg(target_os = "macos")]
fn hour12() -> Option<bool> {
    use objc2_foundation::{NSDateFormatter, NSLocale, NSString};
    // "j" expands to the preferred hour symbol, honoring the System Settings override.
    let format = NSDateFormatter::dateFormatFromTemplate_options_locale(
        &NSString::from_str("j"),
        0,
        Some(&NSLocale::currentLocale()),
    )?;
    Some(twelve_hour_pattern(&format.to_string()))
}

#[cfg(windows)]
fn hour12() -> Option<bool> {
    use windows_sys::Win32::Globalization::{GetLocaleInfoEx, LOCALE_STIMEFORMAT};
    let mut buffer = [0u16; 80];
    // A null locale name is LOCALE_NAME_USER_DEFAULT, including user overrides.
    let written = unsafe {
        GetLocaleInfoEx(
            std::ptr::null(),
            LOCALE_STIMEFORMAT,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    let length = usize::try_from(written).ok()?.checked_sub(1)?;
    Some(twelve_hour_pattern(&String::from_utf16_lossy(
        &buffer[..length],
    )))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn hour12() -> Option<bool> {
    None
}

/// Whether an ICU or Windows time pattern uses a 12-hour symbol outside quotes.
#[cfg_attr(not(any(target_os = "macos", windows, test)), allow(dead_code))]
fn twelve_hour_pattern(pattern: &str) -> bool {
    let mut quoted = false;
    pattern.chars().any(|c| {
        if c == '\'' {
            quoted = !quoted;
        }
        !quoted && matches!(c, 'h' | 'K')
    })
}

#[cfg(test)]
mod tests {
    use super::twelve_hour_pattern;

    #[test]
    fn detects_twelve_hour_patterns() {
        assert!(twelve_hour_pattern("h a"));
        assert!(twelve_hour_pattern("h:mm:ss tt"));
        assert!(!twelve_hour_pattern("HH"));
        assert!(!twelve_hour_pattern("HH:mm 'h'"));
    }
}
