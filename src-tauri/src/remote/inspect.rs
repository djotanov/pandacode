//! The inspector of a task that runs on another computer (docs/remote-access.md, "Remote
//! inspector"): `git_status`, `git_diff`, `list_files` and `read_file` for a mirror ask its host
//! (`git.status`, `task.diff`, `files.list`, `files.read`) and answer in this computer's shapes,
//! so the Changes and Files views work unchanged. Review only: nothing here changes the host.
use super::{client, mirror};
use crate::{workspace, AppState, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The most of one file read here, page by page (the local viewer's limit is the same order).
const MAX_READ: usize = 2 * 1024 * 1024;
/// Pages per read; each is at most the host's page budget.
const MAX_PAGES: usize = 64;

/// The host and host task behind `session`, or None for a local task (or none at all).
fn host_of(state: &AppState, session: Option<&str>) -> Result<Option<mirror::Mirror>> {
    let Some(session) = session else { return Ok(None) };
    let store = state.store.lock().map_err(|e| e.to_string())?;
    mirror::of(&store, session)
}

/// A host's `git.status` reply as this computer's status: one row per path, with its index
/// (staged) and working-tree codes, untracked files as `??`.
pub(crate) fn status_from(reply: &Value) -> workspace::GitStatus {
    if reply["isRepo"] != true {
        return workspace::GitStatus { branch: String::new(), files: Vec::new(), is_repo: false };
    }
    let mut files: BTreeMap<String, workspace::GitFile> = BTreeMap::new();
    let list = |key: &str| reply[key].as_array().cloned().unwrap_or_default();
    let code = |entry: &Value| entry["status"].as_str().filter(|code| code.chars().count() == 1).unwrap_or("M").to_owned();
    fn row<'a>(files: &'a mut BTreeMap<String, workspace::GitFile>, path: &str) -> &'a mut workspace::GitFile {
        files.entry(path.to_owned()).or_insert_with(|| workspace::GitFile { path: path.to_owned(), index: " ".into(), working: " ".into(), orig_path: None })
    }
    for entry in list("staged") {
        let Some(path) = entry["path"].as_str() else { continue };
        let file = row(&mut files, path);
        file.index = code(&entry);
        file.orig_path = entry["origPath"].as_str().map(str::to_owned);
    }
    for entry in list("unstaged") {
        let Some(path) = entry["path"].as_str() else { continue };
        row(&mut files, path).working = code(&entry);
    }
    for entry in list("untracked") {
        let Some(path) = entry.as_str() else { continue };
        let file = row(&mut files, path);
        file.index = "?".into();
        file.working = "?".into();
    }
    workspace::GitStatus { branch: reply["branch"].as_str().unwrap_or_default().to_owned(), files: files.into_values().collect(), is_repo: true }
}

/// A host's `files.list` reply as this computer's entries.
pub(crate) fn entries_from(reply: &Value) -> Vec<workspace::FileEntry> {
    reply["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| Some(workspace::FileEntry { name: entry["name"].as_str()?.to_owned(), path: entry["path"].as_str()?.to_owned(), is_dir: entry["dir"] == true }))
        .collect()
}

/// A host's `task.diff` reply as the patch text, saying what it left out.
pub(crate) fn diff_from(reply: &Value) -> String {
    let mut diff = reply["diff"].as_str().unwrap_or_default().to_owned();
    if let Some(hidden) = reply["hiddenSecrets"].as_u64().filter(|hidden| *hidden > 0) {
        // After the last hunk, where the viewer shows it as trailing text.
        diff.push_str(&format!("\n[{hidden} {} that may hold secrets left out by the other computer]\n", if hidden == 1 { "file" } else { "files" }));
    }
    diff
}

/// `git_status` for a mirror, from its host; None for a local task.
pub(crate) async fn git_status(app: &tauri::AppHandle, state: &AppState, session: Option<&str>) -> Result<Option<workspace::GitStatus>> {
    let Some(host) = host_of(state, session)? else { return Ok(None) };
    let reply = client::call(app, &host.device, "git.status", json!({"sessionId": host.session})).await?;
    Ok(Some(status_from(&reply)))
}

/// `git_diff` for a mirror, from its host; None for a local task.
pub(crate) async fn git_diff(app: &tauri::AppHandle, state: &AppState, session: Option<&str>, path: Option<&str>, staged: bool) -> Result<Option<String>> {
    let Some(host) = host_of(state, session)? else { return Ok(None) };
    let mut params = json!({"sessionId": host.session, "staged": staged});
    if let Some(path) = path {
        params["path"] = json!(path);
    }
    let reply = client::call(app, &host.device, "task.diff", params).await?;
    Ok(Some(diff_from(&reply)))
}

/// `list_files` for a mirror, from its host; None for a local task.
pub(crate) async fn list_files(app: &tauri::AppHandle, state: &AppState, session: Option<&str>, path: Option<&str>) -> Result<Option<Vec<workspace::FileEntry>>> {
    let Some(host) = host_of(state, session)? else { return Ok(None) };
    let mut params = json!({"sessionId": host.session});
    if let Some(path) = path.filter(|path| !path.is_empty()) {
        params["path"] = json!(path);
    }
    let reply = client::call(app, &host.device, "files.list", params).await?;
    Ok(Some(entries_from(&reply)))
}

/// The text of one `files.read` page and whether it was the last; an image or other kind is
/// refused, since only text reaches this viewer.
pub(crate) fn page_text(page: &Value, offset: usize) -> Result<(&str, bool)> {
    if page["kind"] != "text" {
        return Err("This file can't be shown here: open it on the other computer.".into());
    }
    if page["offset"].as_u64() != Some(offset as u64) {
        return Err("The other computer sent an invalid file page.".into());
    }
    Ok((page["text"].as_str().unwrap_or_default(), page["done"] == true))
}

/// `read_file` for a mirror, from its host page by page (up to [`MAX_READ`]); None for a local task.
pub(crate) async fn read_file(app: &tauri::AppHandle, state: &AppState, session: Option<&str>, path: &str) -> Result<Option<String>> {
    let Some(host) = host_of(state, session)? else { return Ok(None) };
    let mut text = String::new();
    for _ in 0..MAX_PAGES {
        let page = client::call(app, &host.device, "files.read", json!({"sessionId": host.session, "path": path, "offset": text.len()})).await?;
        let (chunk, done) = page_text(&page, text.len())?;
        text.push_str(chunk);
        if done || chunk.is_empty() {
            return Ok(Some(text));
        }
        if text.len() >= MAX_READ {
            break;
        }
    }
    text.push_str("\n[Preview truncated: the rest of this file is on the other computer]\n");
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hosts_status_lists_each_path_once_with_both_codes() {
        let reply = json!({"isRepo": true, "branch": "feature", "staged": [{"path": "a.rs", "status": "M"}, {"path": "new.rs", "status": "R", "origPath": "old.rs"}], "unstaged": [{"path": "a.rs", "status": "M"}, {"path": "b.rs", "status": "D"}], "untracked": ["notes.md"]});
        let status = status_from(&reply);
        assert!(status.is_repo);
        assert_eq!(status.branch, "feature");
        let rows: Vec<(&str, &str, &str, Option<&str>)> = status.files.iter().map(|f| (f.path.as_str(), f.index.as_str(), f.working.as_str(), f.orig_path.as_deref())).collect();
        assert_eq!(rows, [("a.rs", "M", "M", None), ("b.rs", " ", "D", None), ("new.rs", "R", " ", Some("old.rs")), ("notes.md", "?", "?", None)]);
    }

    #[test]
    fn a_folder_that_is_not_a_repository_has_no_changes() {
        let status = status_from(&json!({"isRepo": false, "branch": "", "staged": [], "unstaged": [], "untracked": []}));
        assert!(!status.is_repo && status.files.is_empty());
    }

    #[test]
    fn listed_entries_keep_their_kind() {
        let entries = entries_from(&json!({"path": "", "entries": [{"name": "src", "path": "src", "dir": true}, {"name": "README.md", "path": "README.md", "dir": false}, {"bad": 1}]}));
        let rows: Vec<(&str, bool)> = entries.iter().map(|e| (e.path.as_str(), e.is_dir)).collect();
        assert_eq!(rows, [("src", true), ("README.md", false)]);
    }

    #[test]
    fn a_diff_says_how_many_secret_files_were_left_out() {
        assert_eq!(diff_from(&json!({"diff": "diff --git a/x b/x\n", "truncated": false})), "diff --git a/x b/x\n");
        assert!(diff_from(&json!({"diff": "", "hiddenSecrets": 2})).contains("2 files that may hold secrets"));
    }

    #[test]
    fn file_pages_must_be_text_and_follow_on() {
        assert_eq!(page_text(&json!({"kind": "text", "text": "abc", "offset": 0, "done": true}), 0).unwrap(), ("abc", true));
        assert!(page_text(&json!({"kind": "image", "path": "a.png"}), 0).is_err());
        assert!(page_text(&json!({"kind": "text", "text": "abc", "offset": 5, "done": false}), 3).is_err(), "a page from elsewhere in the file");
    }
}
