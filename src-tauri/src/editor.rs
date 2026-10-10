use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::SharedState;

const MAX_DRAFT_BYTES: u64 = 4 * 1024 * 1024;

pub struct EditorSession {
    id: u64,
    path: PathBuf,
    original: String,
    saved: bool,
}

#[derive(Clone, Serialize)]
pub struct EditorCapture {
    pub session_id: u64,
    pub text: String,
}

pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<EditorSession>, String> {
    let args: Vec<_> = args.into_iter().collect();
    if args.is_empty() {
        return Ok(None);
    }
    if args.len() != 2 || args[0] != "--opencode-editor" {
        return Err("Expected --opencode-editor followed by one draft file path.".into());
    }
    EditorSession::open(PathBuf::from(&args[1])).map(Some)
}

impl EditorSession {
    fn open(path: PathBuf) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(&path).map_err(|e| format!("Cannot read the editor draft: {e}"))?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_DRAFT_BYTES {
            return Err("The editor draft must be a regular UTF-8 text file smaller than 4 MiB, not a link.".into());
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let original = fs::read_to_string(&path).map_err(|e| format!("Cannot read the editor draft as UTF-8: {e}"))?;
        Ok(Self { id: crate::optimizer::next_job_id(), path, original, saved: false })
    }

    fn capture(&self) -> EditorCapture {
        EditorCapture { session_id: self.id, text: self.original.clone() }
    }

    fn validate_original(&self) -> Result<(), String> {
        let metadata = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() || self.path.canonicalize().map_err(|e| e.to_string())? != self.path {
            return Err("The original editor file changed location or became a link. Copy your result instead.".into());
        }
        if fs::read_to_string(&self.path).map_err(|e| e.to_string())? != self.original {
            return Err("The original editor draft changed. It was not overwritten; copy your result instead.".into());
        }
        Ok(())
    }

    fn save(&mut self, session_id: u64, text: &str) -> Result<(), String> {
        if self.id != session_id || self.saved {
            return Err("This editor session is stale or already accepted.".into());
        }
        if text.trim().is_empty() || text.len() as u64 > MAX_DRAFT_BYTES {
            return Err("The returned prompt must be nonempty and smaller than 4 MiB.".into());
        }
        self.validate_original()?;
        let parent = self.path.parent().ok_or("Editor draft has no parent folder.")?;
        let pending = parent.join(format!(".promptkitcha-{}-{}.tmp", std::process::id(), self.id));
        let mut file = OpenOptions::new().write(true).create_new(true).open(&pending).map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            self.validate_original()?;
            fs::rename(&pending, &self.path).map_err(|e| format!("Cannot return the editor draft: {e}"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&pending);
        } else {
            self.saved = true;
        }
        result
    }
}

#[tauri::command]
pub fn get_editor_session(state: State<'_, SharedState>) -> Option<EditorCapture> {
    state.editor_session.read().as_ref().map(EditorSession::capture)
}

#[tauri::command]
pub async fn save_editor_prompt(state: State<'_, SharedState>, session_id: u64, text: String) -> Result<(), String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::keystroke::wait_for_modifiers()?;
        let mut session = shared.editor_session.write();
        session.as_mut().ok_or("No external-editor session is active.")?.save(session_id, &text)
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("promptkitcha-editor-test-{}-{}", std::process::id(), crate::optimizer::next_job_id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn draft(&self, text: &str) -> PathBuf {
            let path = self.0.join("prompt with spaces.txt");
            fs::write(&path, text).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn ordinary_startup_has_no_editor_session_and_bad_arguments_are_rejected() {
        assert!(from_args(Vec::<OsString>::new()).unwrap().is_none());
        assert!(from_args([OsString::from("--opencode-editor")]).is_err());
        assert!(from_args([OsString::from("--other"), OsString::from("draft.txt")]).is_err());
    }

    #[test]
    fn full_multiline_draft_is_loaded_and_returned_without_extra_newline() {
        let fixture = Fixture::new();
        let original = "  日本語 🙂\tcode();\r\n".repeat(101) + "FINAL_SENTINEL";
        let path = fixture.draft(&original);
        let mut session = from_args([OsString::from("--opencode-editor"), path.clone().into_os_string()]).unwrap().unwrap();
        assert_eq!(session.capture().text, original);
        let output = crate::prompt_builder::render_grounded(&original, None);
        session.save(session.id, &output).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), output);
        assert!(output.contains(&original));
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
    }

    #[test]
    fn cancel_leaves_original_file_unchanged() {
        let fixture = Fixture::new();
        let path = fixture.draft("Unsent original prompt");
        drop(EditorSession::open(path.clone()).unwrap());
        assert_eq!(fs::read_to_string(path).unwrap(), "Unsent original prompt");
    }

    #[test]
    fn changed_deleted_or_invalid_drafts_are_not_overwritten() {
        let fixture = Fixture::new();
        let path = fixture.draft("Original");
        let mut session = EditorSession::open(path.clone()).unwrap();
        assert!(session.save(session.id + 1, "Wrong session").is_err());
        assert!(session.save(session.id, " \n").is_err());
        fs::write(&path, "Changed elsewhere").unwrap();
        assert!(session.save(session.id, "Replacement").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Changed elsewhere");
        fs::remove_file(&path).unwrap();
        assert!(session.save(session.id, "Replacement").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn directory_binary_and_missing_inputs_are_rejected() {
        let fixture = Fixture::new();
        assert!(EditorSession::open(fixture.0.clone()).is_err());
        let path = fixture.0.join("invalid.txt");
        fs::write(&path, [0xff]).unwrap();
        assert!(EditorSession::open(path).is_err());
        assert!(EditorSession::open(fixture.0.join("missing.txt")).is_err());
    }

    #[test]
    fn duplicate_acceptance_cannot_write_again() {
        let fixture = Fixture::new();
        let path = fixture.draft("Original");
        let mut session = EditorSession::open(path.clone()).unwrap();
        session.save(session.id, "Accepted draft").unwrap();
        assert!(session.save(session.id, "Second draft").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "Accepted draft");
    }
}
