//! Native HWND input for the opt-in Android fixture. Messages are addressed to
//! verified descendants, never injected into the global foreground input queue.
use std::time::{Duration, Instant};
use tauri::Window;
use windows_sys::Win32::{
    Foundation::{GetLastError, SetLastError, HWND, POINT},
    Graphics::Gdi::ScreenToClient,
    System::RemoteDesktop::ProcessIdToSessionId,
    UI::Input::KeyboardAndMouse::IsWindowEnabled,
    UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsChild, IsIconic,
        IsWindowVisible, SendMessageTimeoutW, WindowFromPoint, GUITHREADINFO, SMTO_ABORTIFHUNG,
        SMTO_ERRORONEXIT, WM_CHAR, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    },
};

fn belongs(root: HWND, child: HWND) -> bool {
    !child.is_null() && (child == root || unsafe { IsChild(root, child) } != 0)
}

fn session(pid: u32) -> Result<u32, String> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(pid, &mut session) } == 0 {
        return Err(format!(
            "Cannot verify native input session: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(session)
}

fn context(root: HWND) -> Result<GUITHREADINFO, String> {
    let mut pid = 0;
    if unsafe { GetWindowThreadProcessId(root, &mut pid) } == 0 || pid != std::process::id() {
        return Err("Native input window is not owned by this fixture process".into());
    }
    if unsafe { GetForegroundWindow() } != root
        || unsafe { IsWindowVisible(root) } == 0
        || unsafe { IsWindowEnabled(root) } == 0
        || unsafe { IsIconic(root) } != 0
    {
        return Err("The owned native fixture window must be visible and foreground".into());
    }
    let mut gui: GUITHREADINFO = unsafe { std::mem::zeroed() };
    gui.cbSize = std::mem::size_of_val(&gui) as u32;
    if unsafe { GetGUIThreadInfo(0, &mut gui) } == 0 {
        return Err(format!(
            "Cannot verify native input focus: {}",
            std::io::Error::last_os_error()
        ));
    }
    if gui.hwndActive != root || (!gui.hwndCapture.is_null() && !belongs(root, gui.hwndCapture)) {
        return Err("Native fixture activation or pointer capture changed".into());
    }
    Ok(gui)
}

fn recipient(root: HWND, target: HWND) -> Result<(), String> {
    if !belongs(root, target) || unsafe { IsWindowEnabled(target) } == 0 {
        return Err("Native input recipient is outside the owned window".into());
    }
    // WebView2 child HWNDs may belong to its browser process. Their native
    // parent relationship and session authorize routing; only root must share
    // the fixture application's PID (checked by context before dispatch).
    let mut pid = 0;
    if unsafe { GetWindowThreadProcessId(target, &mut pid) } == 0
        || session(pid)? != session(std::process::id())?
    {
        return Err("Native input recipient is outside the fixture session".into());
    }
    Ok(())
}

fn send(root: HWND, target: HWND, message: u32, word: usize, point: isize) -> Result<(), String> {
    context(root)?;
    recipient(root, target)?;
    let mut result = 0;
    // Only system messages with scalar parameters cross the HWND boundary.
    // No pointer payloads, global keyboard state changes or foreground stealing.
    unsafe {
        SetLastError(0);
    }
    if unsafe {
        SendMessageTimeoutW(
            target,
            message,
            word,
            point,
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            250,
            &mut result,
        )
    } == 0
    {
        return Err(format!(
            "Native input message failed or timed out (Windows error {})",
            unsafe { GetLastError() }
        ));
    }
    Ok(())
}

fn coordinates(x: i32, y: i32) -> Result<isize, String> {
    let x = i16::try_from(x).map_err(|_| "Native pointer x exceeds Windows message bounds")?;
    let y = i16::try_from(y).map_err(|_| "Native pointer y exceeds Windows message bounds")?;
    Ok(((x as u16 as u32) | ((y as u16 as u32) << 16)) as isize)
}

pub(super) fn pointer(window: &Window, phase: &str, x: f64, y: f64) -> Result<(), String> {
    let root = window.hwnd().map_err(|e| e.to_string())?.0 as HWND;
    let gui = context(root)?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let origin = window.inner_position().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let px = (x * scale).round();
    let py = (y * scale).round();
    if !scale.is_finite()
        || scale <= 0.0
        || !px.is_finite()
        || !py.is_finite()
        || px < 0.0
        || py < 0.0
        || px >= f64::from(size.width)
        || py >= f64::from(size.height)
    {
        return Err("Native pointer is outside the physical fixture client area".into());
    }
    let screen = POINT {
        x: origin
            .x
            .checked_add(px as i32)
            .ok_or("Native pointer screen x overflow")?,
        y: origin
            .y
            .checked_add(py as i32)
            .ok_or("Native pointer screen y overflow")?,
    };
    let hit = unsafe { WindowFromPoint(screen) };
    recipient(root, hit)?;
    let target = if gui.hwndCapture.is_null() {
        hit
    } else {
        gui.hwndCapture
    };
    recipient(root, target)?;
    let (message, word, point) = match phase {
        "wheel-up" | "wheel-down" => {
            let delta: i16 = if phase == "wheel-up" { 120 } else { -120 };
            (
                WM_MOUSEWHEEL,
                (u32::from(delta as u16) << 16) as usize,
                coordinates(screen.x, screen.y)?,
            )
        }
        "down" | "up" | "drag" | "move" => {
            let mut client = screen;
            if unsafe { ScreenToClient(target, &mut client) } == 0 {
                return Err("Cannot map native pointer into its recipient".into());
            }
            let message = match phase {
                "down" => WM_LBUTTONDOWN,
                "up" => WM_LBUTTONUP,
                _ => WM_MOUSEMOVE,
            };
            (
                message,
                usize::from(matches!(phase, "down" | "drag")),
                coordinates(client.x, client.y)?,
            )
        }
        _ => return Err("Invalid native pointer phase".into()),
    };
    send(root, target, message, word, point)
}

pub(super) fn text(window: &Window, action: &str, text: &str) -> Result<(), String> {
    if action != "commit" {
        return Err("Native Windows IME composition is not implemented or qualified".into());
    }
    let root = window.hwnd().map_err(|e| e.to_string())?.0 as HWND;
    let focus = context(root)?.hwndFocus;
    recipient(root, focus)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    for unit in text.encode_utf16() {
        if Instant::now() >= deadline {
            return Err("Native Unicode input timed out".into());
        }
        if context(root)?.hwndFocus != focus {
            return Err("Native text focus changed during committed input".into());
        }
        send(root, focus, WM_CHAR, usize::from(unit), 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_foreign_native_recipients_are_rejected_before_dispatch() {
        let missing = std::ptr::null_mut();
        assert!(context(missing).is_err());
        assert!(recipient(missing, missing).is_err());
        let foreground = unsafe { GetForegroundWindow() };
        if !foreground.is_null() {
            // A real foreground HWND is still not a child of a missing owner.
            assert!(recipient(missing, foreground).is_err());
        }
    }

    #[test]
    fn native_message_coordinates_preserve_signed_client_and_screen_values() {
        for (x, y) in [(0, 0), (-32768, 32767), (-120, -40), (1024, 768)] {
            let packed = coordinates(x, y).unwrap() as u32;
            assert_eq!(packed as u16 as i16 as i32, x);
            assert_eq!((packed >> 16) as u16 as i16 as i32, y);
        }
        assert!(coordinates(32768, 0).is_err());
        assert!(coordinates(0, -32769).is_err());
    }
}
