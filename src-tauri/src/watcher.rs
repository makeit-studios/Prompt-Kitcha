use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::SharedState;
use std::sync::atomic::Ordering;

fn should_offer_copy(previous: u32, current: u32, text: &str, self_write: Option<(u32, &str)>) -> bool {
    current != previous && !text.trim().is_empty() && self_write != Some((current, text))
}

pub fn spawn(app: AppHandle, state: SharedState) {
    tauri::async_runtime::spawn(async move {
        let mut last_sequence = crate::clipboard::sequence_number();
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;

            if !state.config.read().show_pill_on_copy {
                last_sequence = crate::clipboard::sequence_number();
                continue;
            }

            let candidate = {
                let written = state.last_self_write.read();
                let sequence = crate::clipboard::sequence_number();
                if sequence == last_sequence { continue; }
                let Ok(text) = app.clipboard().read_text() else { continue };
                if crate::clipboard::sequence_number() != sequence { continue; }
                let offer = should_offer_copy(last_sequence, sequence, &text,
                    written.as_ref().map(|(number, text)| (*number, text.as_str())));
                last_sequence = sequence;
                if offer { Some(text) } else { None }
            };
            let Some(current) = candidate else { continue };
            if state.hotkey_busy.load(Ordering::Acquire) { continue; }

            if let Some(popup) = app.get_webview_window("popup") {
                if popup.is_visible().unwrap_or(false) {
                    continue;
                }
            }

            let hint = crate::foreground::capture();
            if crate::foreground::is_our_process(&hint) { continue; }
            *state.active_app.write() = Some(hint);
            if let Some(pill) = app.get_webview_window("pill") {
                if !pill.is_visible().unwrap_or(false) {
                    if let Ok(pos) = crate::cursor::get_popup_anchor(180, 52) {
                        let _ = pill.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
                    }
                    let _ = pill.show();
                }
            }
            let _ = app.emit("pw:pill-shown", current);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::should_offer_copy;

    #[test]
    fn detects_recopying_identical_text() {
        assert!(should_offer_copy(20, 21, "same prompt", None));
    }

    #[test]
    fn ignores_an_unchanged_clipboard() {
        assert!(!should_offer_copy(20, 20, "prompt", None));
    }

    #[test]
    fn ignores_our_write_but_allows_a_later_external_copy_of_it() {
        let written = Some((20, "prompt"));
        assert!(!should_offer_copy(19, 20, "prompt", written));
        assert!(should_offer_copy(20, 21, "prompt", written));
    }

    #[test]
    fn ignores_empty_and_whitespace_copies() {
        assert!(!should_offer_copy(20, 21, "", None));
        assert!(!should_offer_copy(20, 21, " \n\t", None));
    }
}
