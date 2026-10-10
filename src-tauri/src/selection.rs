use crate::foreground::AppHint;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionIdentity {
    pub focused: Vec<i32>,
    pub owner: Vec<i32>,
    pub prefix_length: usize,
    pub document_hash: u64,
}

#[derive(Clone)]
pub struct SelectionSnapshot {
    pub text: String,
    pub identity: Option<SelectionIdentity>,
    pub editable: bool,
}

fn fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

pub fn validate_snapshot(original: &SelectionSnapshot, current: &SelectionSnapshot) -> Result<(), String> {
    ensure_editable(Some(!current.editable))?;
    if original.identity.is_none() || original.identity != current.identity || original.text != current.text {
        return Err("The original field, document, or selection changed. Select it again or copy the result.".into());
    }
    Ok(())
}

const CLIPBOARD_FALLBACK_ALLOWLIST: &[&str] = &["notepad.exe", "wordpad.exe", "write.exe"];

const HIGH_RISK_PROCESSES: &[&str] = &[
    "code.exe",
    "cursor.exe",
    "trae.exe",
    "windsurf.exe",
    "opencode.exe",
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "wezterm.exe",
    "windowsterminal.exe",
    "powershell.exe",
    "pwsh.exe",
    "cmd.exe",
];

pub fn clipboard_fallback_allowed(hint: &AppHint) -> bool {
    let process = hint.process.trim().to_ascii_lowercase();
    if HIGH_RISK_PROCESSES.iter().any(|name| process == *name)
        || process.contains("electron")
    {
        return false;
    }
    CLIPBOARD_FALLBACK_ALLOWLIST
        .iter()
        .any(|name| process == *name)
}

#[cfg(windows)]
pub fn selected_snapshot(require_editable: bool) -> Result<Option<SelectionSnapshot>, String> {
    use uiautomation::patterns::UITextPattern;
    use uiautomation::types::{TextAttribute, TextPatternRangeEndpoint};
    use uiautomation::variants::Value;
    use uiautomation::UIAutomation;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("UI Automation thread initialization failed: {e}"))?;
    let _apartment = Apartment;

    let automation = UIAutomation::new_direct().map_err(|e| format!("UI Automation unavailable: {e}"))?;
    let walker = automation
        .get_control_view_walker()
        .map_err(|e| format!("UI Automation tree unavailable: {e}"))?;
    let mut element = automation
        .get_focused_element()
        .map_err(|e| format!("focused control unavailable: {e}"))?;
    let focused = element.get_runtime_id().ok();

    for depth in 0..10 {
        if element.is_password().unwrap_or(true) {
            return Err("Password fields cannot be captured.".into());
        }
        if let Ok(pattern) = element.get_pattern::<UITextPattern>() {
            if let Ok(ranges) = pattern.get_selection() {
                if ranges.len() > 1 {
                    return Err("Select one continuous range of text before enhancing.".into());
                }
                for range in ranges {
                    if let Ok(text) = range.get_text(-1) {
                        if !text.trim().is_empty() {
                            let read_only = match range
                                .get_attribute_value(TextAttribute::IsReadOnly)
                                .and_then(|attribute| attribute.get_value())
                            {
                                Ok(Value::BOOL(value)) => Some(value),
                                _ => None,
                            };
                            let identity = (|| {
                                let focused = focused.clone().filter(|id| !id.is_empty()).ok_or(())?;
                                let owner = element.get_runtime_id().map_err(|_| ())?;
                                if owner.is_empty() { return Err(()); }
                                let document = pattern.get_document_range().map_err(|_| ())?;
                                let document_text = document.get_text(-1).map_err(|_| ())?;
                                document.move_endpoint_by_range(TextPatternRangeEndpoint::End, &range, TextPatternRangeEndpoint::Start).map_err(|_| ())?;
                                let prefix = document.get_text(-1).map_err(|_| ())?;
                                Ok::<_, ()>(SelectionIdentity {
                                    focused, owner,
                                    prefix_length: prefix.encode_utf16().count(),
                                    document_hash: fingerprint(&document_text),
                                })
                            })().ok();
                            if require_editable { ensure_editable(read_only)?; }
                            return Ok(Some(SelectionSnapshot { text, identity, editable: read_only == Some(false) }));
                        }
                    }
                }
                return Ok(None);
            }
        }

        if depth == 9 {
            break;
        }
        match walker.get_parent(&element) {
            Ok(parent) => element = parent,
            Err(_) => break,
        }
    }

    Ok(None)
}

#[cfg(not(windows))]
pub fn selected_snapshot(_require_editable: bool) -> Result<Option<SelectionSnapshot>, String> {
    Ok(None)
}

#[cfg(windows)]
pub fn native_edit_identity() -> Result<SelectionIdentity, String> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::Controls::EM_GETSEL;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetGUIThreadInfo, GetWindowLongPtrW, GetWindowThreadProcessId,
        SendMessageTimeoutW, GUITHREADINFO, GWL_STYLE, SMTO_ABORTIFHUNG, SMTO_BLOCK,
        WM_GETTEXT, WM_GETTEXTLENGTH,
    };
    unsafe {
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(0, &mut info).map_err(|e| e.to_string())?;
        let hwnd = info.hwndFocus;
        if hwnd.0.is_null() { return Err("The focused editor is unavailable.".into()); }
        let mut class = [0u16; 128];
        let length = GetClassNameW(hwnd, &mut class);
        let class = String::from_utf16_lossy(&class[..length.max(0) as usize]).to_lowercase();
        if !matches!(class.as_str(), "edit" | "richedit20w" | "richedit50w" | "richeditd2dpt") {
            return Err("Windows cannot confirm this editor's selection range. Copy the result instead.".into());
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if style & (0x20 | 0x800) != 0 {
            return Err("Password or read-only fields cannot be replaced.".into());
        }
        let flags = SMTO_ABORTIFHUNG | SMTO_BLOCK;
        let mut start = u32::MAX;
        let mut end = u32::MAX;
        if SendMessageTimeoutW(hwnd, EM_GETSEL, WPARAM(&mut start as *mut u32 as usize), LPARAM(&mut end as *mut u32 as isize), flags, 300, None).0 == 0 || start >= end {
            return Err("No continuous native editor selection is available.".into());
        }
        let mut length = 0usize;
        if SendMessageTimeoutW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0), flags, 300, Some(&mut length)).0 == 0 || length > 8_000_000 {
            return Err("The editor document could not be safely validated. Copy the result instead.".into());
        }
        let mut text = vec![0u16; length + 1];
        let mut copied = 0usize;
        if SendMessageTimeoutW(hwnd, WM_GETTEXT, WPARAM(text.len()), LPARAM(text.as_mut_ptr() as isize), flags, 300, Some(&mut copied)).0 == 0 || copied > length || end as usize > copied {
            return Err("The editor document changed during validation.".into());
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let handle = hwnd.0 as usize as u64;
        let owner = vec![pid as i32, handle as i32, (handle >> 32) as i32, end as i32];
        Ok(SelectionIdentity {
            focused: owner.clone(), owner,
            prefix_length: start as usize,
            document_hash: fingerprint(&String::from_utf16_lossy(&text[..copied])),
        })
    }
}

#[cfg(not(windows))]
pub fn native_edit_identity() -> Result<SelectionIdentity, String> {
    Err("Windows is required.".into())
}

fn ensure_editable(read_only: Option<bool>) -> Result<(), String> {
    if read_only == Some(false) {
        Ok(())
    } else {
        Err("The selection is read-only or Windows cannot confirm it is editable. Copy the enhanced prompt instead.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{clipboard_fallback_allowed, ensure_editable, fingerprint, validate_snapshot, SelectionIdentity, SelectionSnapshot};
    use crate::foreground::AppHint;

    fn hint(process: &str) -> AppHint {
        AppHint {
            process: process.to_string(),
            ..AppHint::default()
        }
    }

    #[test]
    fn allows_known_native_editors() {
        assert!(clipboard_fallback_allowed(&hint("notepad.exe")));
        assert!(clipboard_fallback_allowed(&hint("WORDPAD.EXE")));
    }

    #[test]
    fn blocks_apps_where_copy_without_selection_can_capture_content() {
        for process in [
            "Code.exe",
            "cursor.exe",
            "chrome.exe",
            "WindowsTerminal.exe",
            "pwsh.exe",
        ] {
            assert!(!clipboard_fallback_allowed(&hint(process)), "{process}");
        }
    }

    #[test]
    fn unknown_apps_do_not_get_a_clipboard_guess() {
        assert!(!clipboard_fallback_allowed(&hint("unknown.exe")));
    }

    #[test]
    fn replacement_requires_a_confirmed_editable_range() {
        assert!(ensure_editable(Some(false)).is_ok());
        assert!(ensure_editable(Some(true)).is_err());
        assert!(ensure_editable(None).is_err());
    }

    #[test]
    fn identical_text_in_a_different_field_or_range_cannot_be_replaced() {
        let original = SelectionSnapshot {
            text: "same text".into(),
            identity: Some(SelectionIdentity { focused: vec![1], owner: vec![2], prefix_length: 10, document_hash: fingerprint("before same text after") }),
            editable: true,
        };
        assert!(validate_snapshot(&original, &original).is_ok());
        for change in 0..5 {
            let mut current = original.clone();
            match change {
                0 => current.identity.as_mut().unwrap().focused = vec![3],
                1 => current.identity.as_mut().unwrap().owner = vec![3],
                2 => current.identity.as_mut().unwrap().prefix_length = 30,
                3 => current.identity.as_mut().unwrap().document_hash = fingerprint("different document"),
                _ => current.text = "changed text".into(),
            }
            assert!(validate_snapshot(&original, &current).is_err());
        }
        let mut current = original.clone();
        current.editable = false;
        assert!(validate_snapshot(&original, &current).is_err());
        current.editable = true;
        current.identity = None;
        assert!(validate_snapshot(&original, &current).is_err());
    }
}
