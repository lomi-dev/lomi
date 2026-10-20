//! Bounded Return key routing for the isolated terminal regression fixture.
//! Printable text continues to use the existing owned HWND WM_CHAR helper.
use tauri::Window;
use windows_sys::Win32::{
    Foundation::{GetLastError, SetLastError, HWND},
    System::RemoteDesktop::ProcessIdToSessionId,
    UI::Input::KeyboardAndMouse::{IsWindowEnabled, VK_RETURN},
    UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsChild, IsIconic,
        IsWindowVisible, SendMessageTimeoutW, GUITHREADINFO, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT,
        WM_KEYDOWN, WM_KEYUP,
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

pub(super) fn enter(window: &Window) -> Result<(), String> {
    let root = window.hwnd().map_err(|e| e.to_string())?.0 as HWND;
    let focus = context(root)?.hwndFocus;
    recipient(root, focus)?;
    // Enter has scan code 0x1c; release includes previous-state and transition
    // bits. Scalar system messages target the same verified focused child.
    for (message, flags) in [(WM_KEYDOWN, 0x001c0001u32), (WM_KEYUP, 0xc01c0001u32)] {
        if context(root)?.hwndFocus != focus {
            return Err("Native Return focus changed".into());
        }
        send(root, focus, message, usize::from(VK_RETURN), flags as isize)?;
    }
    Ok(())
}
