use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::SharedState;

pub fn sequence_number() -> u32 {
    #[cfg(windows)]
    unsafe {
        windows::Win32::System::DataExchange::GetClipboardSequenceNumber()
    }
    #[cfg(not(windows))]
    { 0 }
}

pub fn write_tracked(app: &AppHandle, state: &SharedState, text: &str) -> Result<(), String> {
    let mut written = state.last_self_write.write();
    *written = None;
    app.clipboard().write_text(text.to_string()).map_err(|e| e.to_string())?;
    *written = Some((sequence_number(), text.to_string()));
    Ok(())
}

#[tauri::command]
pub fn save_clipboard(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    let text = app.clipboard().read_text().ok();
    *state.saved_clipboard.write() = text;
    Ok(())
}

#[tauri::command]
pub fn restore_clipboard(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    let saved = state.saved_clipboard.write().take();
    if let Some(t) = saved {
        write_tracked(&app, state.inner(), &t)?;
    }
    Ok(())
}

#[tauri::command]
pub fn read_clipboard(app: AppHandle) -> Result<String, String> {
    app.clipboard().read_text().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_clipboard(app: AppHandle, state: State<'_, SharedState>, text: String) -> Result<(), String> {
    write_tracked(&app, state.inner(), &text)
}
