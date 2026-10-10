use std::time::{Duration, Instant};

use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::SharedState;

#[derive(Clone)]
pub struct CapturedSelection {
    pub id: u64,
    pub text: String,
    pub hwnd: isize,
    pub native: bool,
    pub snapshot: crate::selection::SelectionSnapshot,
}

#[derive(Clone, serde::Serialize)]
pub struct SelectionCapture {
    pub capture_id: u64,
    pub replaceable: bool,
    pub text: String,
}

#[cfg(windows)]
mod sys {
    use windows::Win32::System::DataExchange::CountClipboardFormats;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_V,
    };

    fn make(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn chord(second: VIRTUAL_KEY) -> Result<(), String> {
        let inputs = [
            make(VK_CONTROL, false),
            make(second, false),
            make(second, true),
            make(VK_CONTROL, true),
        ];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent != inputs.len() as u32 {
            return Err("Windows blocked keyboard input to the target application.".into());
        }
        Ok(())
    }

    pub fn modifiers_held() -> bool {
        [0x0D, 0x10, 0x11, 0x12, 0x5B, 0x5C]
            .iter()
            .any(|key| unsafe { GetAsyncKeyState(*key) < 0 })
    }

    pub fn clipboard_empty() -> bool {
        unsafe { CountClipboardFormats() == 0 }
    }

    pub fn send_copy() -> Result<(), String> { chord(VK_C) }
    pub fn send_paste() -> Result<(), String> { chord(VK_V) }
}

#[cfg(not(windows))]
mod sys {
    pub fn modifiers_held() -> bool { false }
    pub fn clipboard_empty() -> bool { false }
    pub fn send_copy() -> Result<(), String> { Err("Windows is required.".into()) }
    pub fn send_paste() -> Result<(), String> { Err("Windows is required.".into()) }
}

pub(crate) fn wait_for_modifiers() -> Result<(), String> {
    let started = Instant::now();
    while sys::modifiers_held() {
        if started.elapsed() > Duration::from_secs(2) {
            return Err("Release Enter and the shortcut keys, then try again.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn clipboard_snapshot(app: &AppHandle) -> Result<String, String> {
    match app.clipboard().read_text() {
        Ok(text) => Ok(text),
        Err(_) if sys::clipboard_empty() => Ok(String::new()),
        Err(_) => Err("Your clipboard contains non-text data. Save it elsewhere before using Replace. Copying the result will overwrite it.".into()),
    }
}

fn write_owned(app: &AppHandle, state: &SharedState, text: &str) -> Result<(), String> {
    crate::clipboard::write_tracked(app, state, text)
}

fn restore_if_owned(app: &AppHandle, state: &SharedState, owned: &str, prior: &str) {
    if app.clipboard().read_text().ok().as_deref() == Some(owned) {
        let _ = write_owned(app, state, prior);
    }
}

fn clipboard_capture(app: &AppHandle, state: &SharedState, hwnd: isize) -> Result<String, String> {
    wait_for_modifiers()?;
    if crate::foreground::capture().hwnd != hwnd {
        return Err("The active window changed before capture. Select the text again.".into());
    }
    let prior = clipboard_snapshot(app)?;
    let marker = format!("\u{E000}promptkitcha-{}\u{E001}", crate::optimizer::next_job_id());
    write_owned(app, state, &marker)?;
    let captured = (|| {
        sys::send_copy()?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(350) {
            std::thread::sleep(Duration::from_millis(10));
            if let Ok(text) = app.clipboard().read_text() {
                if text != marker {
                    return Ok(text);
                }
            }
        }
        Err("No text selection was captured. Select text and try again.".to_string())
    })();
    let owned = captured.as_ref().map(|text| text.as_str()).unwrap_or(&marker);
    restore_if_owned(app, state, owned, &prior);
    captured
}

#[tauri::command]
pub async fn capture_selection(app: AppHandle, state: State<'_, SharedState>) -> Result<SelectionCapture, String> {
    let shared = state.inner().clone();
    *shared.captured_selection.write() = None;
    tauri::async_runtime::spawn_blocking(move || {
        let hint = crate::foreground::capture();
        if hint.hwnd == 0 || crate::foreground::is_our_process(&hint) {
            return Err("Select text in the target application and press the shortcut.".into());
        }
        let (snapshot, native) = match crate::selection::selected_snapshot(false)? {
            Some(snapshot) => (snapshot, true),
            None if crate::selection::clipboard_fallback_allowed(&hint) => {
                let before = crate::selection::native_edit_identity().ok();
                let text = clipboard_capture(&app, &shared, hint.hwnd)?;
                let after = crate::selection::native_edit_identity().ok();
                let identity = before.filter(|identity| Some(identity) == after.as_ref());
                let editable = identity.is_some();
                (crate::selection::SelectionSnapshot { text, identity, editable }, false)
            }
            None => return Err(format!(
                "No selection is available in {}. In terminals, copy the prompt yourself, then choose Use copied text. No Ctrl+C is sent to the terminal. In VS Code's editor, enabling accessibility support may help.", hint.process
            )),
        };
        let text = snapshot.text.clone();
        if text.trim().is_empty() || crate::foreground::capture().hwnd != hint.hwnd {
            return Err("The selection is empty or the active window changed. Select the text again.".into());
        }
        *shared.active_app.write() = Some(hint.clone());
        let capture_id = crate::optimizer::next_job_id();
        let replaceable = snapshot.editable && snapshot.identity.is_some();
        *shared.captured_selection.write() = Some(CapturedSelection { id: capture_id, text: text.clone(), hwnd: hint.hwnd, native, snapshot });
        Ok(SelectionCapture { capture_id, text, replaceable })
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn replace_selection(app: AppHandle, state: State<'_, SharedState>, text: String, capture_id: u64) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("There is no enhanced prompt to replace the selection with.".into());
    }
    let shared = state.inner().clone();
    let captured = shared.captured_selection.read().clone()
        .ok_or("No replaceable selection was captured. Copy the result instead.")?;
    ensure_capture_id(captured.id, capture_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        wait_for_modifiers()?;
        if !crate::foreground::focus_hwnd(captured.hwnd) {
            return Err("The original application could not be focused. Copy the result instead.".into());
        }
        std::thread::sleep(Duration::from_millis(40));
        let current = if captured.native {
            crate::selection::selected_snapshot(true)?
                .ok_or("The original selection is no longer available.")?
        } else {
            let identity = crate::selection::native_edit_identity()?;
            let text = clipboard_capture(&app, &shared, captured.hwnd)?;
            crate::selection::SelectionSnapshot { text, identity: Some(identity), editable: true }
        };
        crate::selection::validate_snapshot(&captured.snapshot, &current)?;
        if current.text != captured.text || crate::foreground::capture().hwnd != captured.hwnd {
            return Err("The original selection changed. Select it again or copy the result.".into());
        }
        let mut selection = shared.captured_selection.write();
        ensure_capture_id(selection.as_ref().map(|capture| capture.id).unwrap_or(0), capture_id)?;
        let prior = clipboard_snapshot(&app)?;
        write_owned(&app, &shared, &text)?;
        let validation = if captured.native {
            crate::selection::selected_snapshot(true).and_then(|current| {
                let current = current.ok_or("The original selection is no longer available.")?;
                crate::selection::validate_snapshot(&captured.snapshot, &current)
            })
        } else {
            crate::selection::native_edit_identity().and_then(|identity| {
                if Some(identity) == captured.snapshot.identity { Ok(()) }
                else { Err("The original editor selection changed before Paste.".into()) }
            })
        };
        if let Err(error) = validation {
            restore_if_owned(&app, &shared, &text, &prior);
            return Err(error);
        }
        if crate::foreground::capture().hwnd != captured.hwnd {
            restore_if_owned(&app, &shared, &text, &prior);
            return Err("The active window changed before Paste. Copy the result instead.".into());
        }
        let result = sys::send_paste();
        std::thread::sleep(Duration::from_millis(200));
        restore_if_owned(&app, &shared, &text, &prior);
        result?;
        *selection = None;
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

fn ensure_capture_id(current: u64, expected: u64) -> Result<(), String> {
    if current != 0 && current == expected {
        Ok(())
    } else {
        Err("This draft belongs to an older selection. Select the text again or copy the result.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_capture_id;

    #[test]
    fn replacement_is_bound_to_the_original_capture() {
        assert!(ensure_capture_id(41, 41).is_ok());
        assert!(ensure_capture_id(42, 41).is_err());
        assert!(ensure_capture_id(0, 0).is_err());
        assert!(ensure_capture_id(0, 41).is_err());
    }
}
