//! Android input follows native foreground/focus ownership on Windows. WebView2
//! may focus a child HWND in another process without updating Tao's focus cache.
use tauri::Window;
use windows_sys::Win32::{
    Foundation::HWND,
    System::RemoteDesktop::ProcessIdToSessionId,
    UI::Input::KeyboardAndMouse::IsWindowEnabled,
    UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsChild, IsIconic,
        IsWindowVisible, GUITHREADINFO,
    },
};

fn session(pid: u32) -> Result<u32, String> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(pid, &mut session) } == 0 {
        return Err(format!(
            "Cannot verify Android window session: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(session)
}

fn owns_focus(root: HWND, active: HWND, focus: HWND) -> Result<bool, String> {
    if active != root
        || focus.is_null()
        || (focus != root && unsafe { IsChild(root, focus) } == 0)
        || unsafe { IsWindowEnabled(focus) } == 0
    {
        return Ok(false);
    }
    // A WebView2 recipient may have a different PID. Its actual native child
    // relationship and session identify it; only the root must share our PID.
    let mut pid = 0;
    if unsafe { GetWindowThreadProcessId(focus, &mut pid) } == 0 {
        return Err(format!(
            "Cannot verify Android native focus: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(session(pid)? == session(std::process::id())?)
}

fn native_focused(root: HWND) -> Result<bool, String> {
    let mut pid = 0;
    if root.is_null() || unsafe { GetWindowThreadProcessId(root, &mut pid) } == 0 {
        return Ok(false);
    }
    if pid != std::process::id()
        || unsafe { GetForegroundWindow() } != root
        || unsafe { IsWindowVisible(root) } == 0
        || unsafe { IsWindowEnabled(root) } == 0
        || unsafe { IsIconic(root) } != 0
    {
        return Ok(false);
    }
    let mut gui: GUITHREADINFO = unsafe { std::mem::zeroed() };
    gui.cbSize = std::mem::size_of_val(&gui) as u32;
    if unsafe { GetGUIThreadInfo(0, &mut gui) } == 0 {
        return Err(format!(
            "Cannot inspect Android native focus: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(
        owns_focus(root, gui.hwndActive, gui.hwndFocus)?
            && unsafe { GetForegroundWindow() } == root,
    )
}

pub(super) fn is_focused(window: &Window) -> Result<bool, String> {
    if window.label() != "main" {
        return Ok(false);
    }
    native_focused(window.hwnd().map_err(|error| error.to_string())?.0 as HWND)
}
