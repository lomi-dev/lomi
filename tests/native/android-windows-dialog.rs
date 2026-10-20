//! Addressed control messages for the fixture's own native file dialog.
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tauri::Window;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    System::{
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{AttachThreadInput, GetCurrentThreadId},
    },
    UI::{
        Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled, SetFocus},
        WindowsAndMessaging::{
            EnumChildWindows, GetClassNameW, GetDlgCtrlID, GetDlgItem, GetForegroundWindow,
            GetParent, GetWindow, GetWindowTextW, GetWindowThreadProcessId, IsChild,
            IsWindowVisible, PeekMessageW, PostMessageW, SendMessageTimeoutW, BM_CLICK, GW_OWNER,
            IDCANCEL, IDOK, MSG, PM_NOREMOVE, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, WM_CHAR,
            WM_CLEAR,
        },
    },
};

// Standard Win32 common-dialog filename controls (cmb13 and edt1) and EM_SETSEL.
// Deliberately do not guess at other Edit controls such as the address/search box.
const FILENAME_COMBO: i32 = 1148;
const FILENAME_EDIT: i32 = 1152;
const EM_SETSEL: u32 = 0x00b1;

fn class(window: HWND) -> String {
    let mut text = [0; 128];
    let count = unsafe { GetClassNameW(window, text.as_mut_ptr(), text.len() as i32) };
    String::from_utf16_lossy(&text[..count.max(0) as usize])
}

fn owned(window: HWND) -> Result<(), String> {
    let mut pid = 0;
    let mut session = 0;
    let mut current_session = 0;
    if window.is_null()
        || unsafe { GetWindowThreadProcessId(window, &mut pid) } == 0
        || pid != std::process::id()
        || unsafe { ProcessIdToSessionId(pid, &mut session) } == 0
        || unsafe { ProcessIdToSessionId(std::process::id(), &mut current_session) } == 0
        || session != current_session
    {
        return Err("Native file dialog is outside the fixture process/session".into());
    }
    Ok(())
}

fn context(main: HWND, dialog: HWND) -> Result<(), String> {
    owned(main)?;
    owned(dialog)?;
    if dialog == main
        || unsafe { GetForegroundWindow() } != dialog
        || unsafe { IsWindowVisible(dialog) } == 0
        || unsafe { IsWindowEnabled(dialog) } == 0
        || class(dialog) != "#32770"
    {
        return Err("The owned native file dialog must be visible and foreground".into());
    }
    let mut owner = dialog;
    for _ in 0..16 {
        owner = unsafe { GetWindow(owner, GW_OWNER) };
        owned(owner)?;
        if owner == main {
            return Ok(());
        }
    }
    Err("Native file dialog owner chain does not reach main".into())
}

fn recipient(main: HWND, dialog: HWND, target: HWND) -> Result<(), String> {
    context(main, dialog)?;
    owned(target)?;
    if unsafe { IsChild(dialog, target) } == 0
        || unsafe { IsWindowVisible(target) } == 0
        || unsafe { IsWindowEnabled(target) } == 0
    {
        return Err("Native dialog control is not an enabled visible descendant".into());
    }
    Ok(())
}

unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> i32 {
    let children = unsafe { &mut *(data as *mut Vec<HWND>) };
    if children.len() >= 256 {
        return 0;
    }
    children.push(window);
    1
}

fn modern_save_filename(main: HWND, dialog: HWND, edit: HWND) -> bool {
    // The observed Windows 11 Save dialog uses this exact filename ancestry.
    // ID 1001 alone is also used by unrelated common-dialog controls.
    if (unsafe { GetDlgCtrlID(edit) }) != 1001 || text(dialog) != "Save Android screenshot" {
        return false;
    }
    let mut parent = edit;
    for expected in [
        "ComboBox",
        "FloatNotifySink",
        "DirectUIHWND",
        "DUIViewWndClassName",
    ] {
        parent = unsafe { GetParent(parent) };
        if recipient(main, dialog, parent).is_err() || class(parent) != expected {
            return false;
        }
        if expected == "ComboBox" && (unsafe { GetDlgCtrlID(parent) }) != 0 {
            return false;
        }
    }
    (unsafe { GetParent(parent) }) == dialog
}

fn filename(main: HWND, dialog: HWND) -> Result<HWND, String> {
    let mut children = Vec::new();
    unsafe {
        EnumChildWindows(
            dialog,
            Some(collect),
            &mut children as *mut Vec<HWND> as LPARAM,
        )
    };
    if children.len() >= 256 {
        return Err("Native file dialog exceeds the fixture control limit".into());
    }
    let candidates: Vec<_> = children
        .iter()
        .copied()
        .filter(|&edit| {
            if class(edit) != "Edit" || recipient(main, dialog, edit).is_err() {
                return false;
            }
            (unsafe { GetDlgCtrlID(edit) }) == FILENAME_EDIT
                || children.iter().any(|&combo| {
                    (unsafe { GetDlgCtrlID(combo) }) == FILENAME_COMBO
                        && matches!(class(combo).as_str(), "ComboBox" | "ComboBoxEx32")
                        && unsafe { IsChild(combo, edit) } != 0
                })
                || modern_save_filename(main, dialog, edit)
        })
        .collect();
    if candidates.len() != 1 {
        return Err(format!(
            "Native file dialog exposes {} recognized filename Edit controls; expected exactly one",
            candidates.len()
        ));
    }
    Ok(candidates[0])
}

/// The fixture types synchronously on this thread. Keep its temporary input
/// queue attachment alive through every scalar message and detach on all exits.
struct FilenameInput {
    current_thread: u32,
    dialog_thread: u32,
    attached: bool,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl FilenameInput {
    fn focus(main: HWND, dialog: HWND, edit: HWND) -> Result<Self, String> {
        recipient(main, dialog, edit)?;
        if filename(main, dialog)? != edit {
            return Err("Native filename recipient changed before focus".into());
        }
        let current_thread = unsafe { GetCurrentThreadId() };
        let dialog_thread = unsafe { GetWindowThreadProcessId(dialog, std::ptr::null_mut()) };
        if dialog_thread == 0 {
            return Err("Cannot verify the native file dialog input thread".into());
        }
        let attached = current_thread != dialog_thread;
        if attached {
            // AttachThreadInput requires a message queue on both threads. Peek
            // only initializes ours; it does not consume posted UI messages.
            let mut message: MSG = unsafe { std::mem::zeroed() };
            unsafe {
                PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            }
            if unsafe { AttachThreadInput(current_thread, dialog_thread, 1) } == 0 {
                return Err(format!(
                    "Cannot attach the verified dialog input queue: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        let input = Self {
            current_thread,
            dialog_thread,
            attached,
            _thread: std::marker::PhantomData,
        };
        // SetFocus may activate its parent, so recheck the already-foreground
        // owned dialog immediately before and after focusing its filename.
        recipient(main, dialog, edit)?;
        unsafe {
            SetFocus(edit);
        }
        input.check(main, dialog, edit)?;
        Ok(input)
    }

    fn check(&self, main: HWND, dialog: HWND, edit: HWND) -> Result<(), String> {
        recipient(main, dialog, edit)?;
        if unsafe { GetFocus() } != edit {
            return Err("The verified native filename control does not have input focus".into());
        }
        Ok(())
    }
}

impl Drop for FilenameInput {
    fn drop(&mut self) {
        if self.attached {
            unsafe {
                AttachThreadInput(self.current_thread, self.dialog_thread, 0);
            }
        }
    }
}

fn send(
    input: &FilenameInput,
    main: HWND,
    dialog: HWND,
    edit: HWND,
    message: u32,
    word: usize,
    value: isize,
) -> Result<(), String> {
    input.check(main, dialog, edit)?;
    let mut result = 0;
    if unsafe {
        SendMessageTimeoutW(
            edit,
            message,
            word,
            value,
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            250,
            &mut result,
        )
    } == 0
    {
        return Err("Native filename input failed or timed out".into());
    }
    Ok(())
}

fn text(edit: HWND) -> String {
    let mut text = vec![0; 32769];
    let count = unsafe { GetWindowTextW(edit, text.as_mut_ptr(), text.len() as i32) };
    String::from_utf16_lossy(&text[..count.max(0) as usize])
}

fn filename_path(path: &Path) -> Result<&str, String> {
    // Canonical paths remain internal. The native dialog must receive the same
    // compatible external spelling used for its exact fixed-path confirmation.
    let path = dunce::simplified(path)
        .to_str()
        .ok_or("Fixture dialog path is not Unicode")?;
    if !Path::new(path).is_absolute() || path.encode_utf16().count() > 32767 {
        return Err("Invalid fixture dialog path".into());
    }
    Ok(path)
}

fn confirmed_filename(selected: &str, screenshot: &Path) -> Result<bool, String> {
    let apk = screenshot
        .parent()
        .ok_or("Missing fixture product directory")?
        .join("apk-selection")
        .join("input-test.apk");
    Ok(selected == filename_path(screenshot)? || selected == filename_path(&apk)?)
}

pub(super) fn control(
    window: &Window,
    action: &str,
    path: &Path,
) -> Result<serde_json::Value, String> {
    crate::files::main_window(window)?;
    let main = window.hwnd().map_err(|e| e.to_string())?.0 as HWND;
    let dialog = unsafe { GetForegroundWindow() };
    context(main, dialog)?;
    let edit = filename(main, dialog)?;
    match action {
        "dialog-state" => {
            recipient(main, dialog, edit)?;
            // This HWND interface exposes the filename, not a reliable folder
            // identity. Full absolute fixture paths avoid depending on that folder.
            return Ok(serde_json::json!({"name":text(edit),"directory":null}));
        }
        "dialog-type-apk" | "dialog-type-screenshot" => {
            let path = filename_path(path)?;
            let input = FilenameInput::focus(main, dialog, edit)?;
            let deadline = Instant::now() + Duration::from_secs(3);
            // Scalar messages avoid pointer lifetime hazards on timeout and
            // target only the verified focused control, not desktop input.
            send(&input, main, dialog, edit, EM_SETSEL, 0, -1)?;
            send(&input, main, dialog, edit, WM_CLEAR, 0, 0)?;
            for unit in path.encode_utf16() {
                if Instant::now() >= deadline {
                    return Err("Native filename input exceeded three seconds".into());
                }
                send(&input, main, dialog, edit, WM_CHAR, usize::from(unit), 1)?;
            }
            input.check(main, dialog, edit)?;
            if text(edit) != path {
                return Err("Native dialog filename does not match the fixture path".into());
            }
        }
        "dialog-confirm" | "dialog-cancel" => {
            if action == "dialog-confirm" {
                recipient(main, dialog, edit)?;
                if !confirmed_filename(&text(edit), path)? {
                    return Err(
                        "Native file dialog filename is outside the two fixed fixture paths".into(),
                    );
                }
            }
            let id = if action == "dialog-confirm" {
                IDOK
            } else {
                IDCANCEL
            };
            let button = unsafe { GetDlgItem(dialog, id) };
            recipient(main, dialog, button)?;
            if class(button) != "Button" {
                return Err("Native file dialog has no recognized action button".into());
            }
            // Queue the targeted button click because accepting a save may open
            // another modal confirmation. The fixture observes completion later.
            if unsafe { PostMessageW(button, BM_CLICK, 0, 0) } == 0 {
                return Err("Could not queue the native file dialog action".into());
            }
        }
        _ => return Err("Unknown Windows file dialog fixture action".into()),
    }
    Ok(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_and_confirmed_filenames_share_only_the_two_compatible_fixed_paths() {
        let screenshot = Path::new(r"\\?\C:\t\private Android żółć\product\screenshot.png");
        let external = r"C:\t\private Android żółć\product\screenshot.png";
        assert_eq!(filename_path(screenshot).unwrap(), external);
        assert!(confirmed_filename(external, screenshot).unwrap());
        assert!(confirmed_filename(
            r"C:\t\private Android żółć\product\apk-selection\input-test.apk",
            screenshot
        )
        .unwrap());
        assert!(
            !confirmed_filename(r"C:\t\private Android żółć\product\other.png", screenshot)
                .unwrap()
        );
        assert!(!confirmed_filename(screenshot.to_str().unwrap(), screenshot).unwrap());
        assert!(filename_path(Path::new("screenshot.png")).is_err());
        let unc = Path::new(r"\\server\share\product\screenshot.png");
        assert_eq!(filename_path(unc).unwrap(), unc.to_str().unwrap());
    }

    #[test]
    fn filename_focus_rejects_unverified_windows_before_attaching_input_queues() {
        let missing = std::ptr::null_mut();
        let before = unsafe { GetFocus() };
        assert!(FilenameInput::focus(missing, missing, missing).is_err());
        assert!(FilenameInput::focus(missing, unsafe { GetForegroundWindow() }, missing).is_err());
        assert_eq!(unsafe { GetFocus() }, before);
    }

    #[test]
    fn missing_windows_and_unrelated_foreground_are_rejected() {
        let missing = std::ptr::null_mut();
        assert!(owned(missing).is_err());
        assert!(context(missing, unsafe { GetForegroundWindow() }).is_err());
        assert!(recipient(missing, missing, missing).is_err());
    }
}
