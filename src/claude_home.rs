//! Claude Code's own folder, `~/.claude`: every project's transcripts under
//! `projects/`, the personal skills, the enabled marketplace plugins.
//!
//! A sandboxed plugin cannot see `$HOME`, so the folder is the `claudeFolder`
//! setting, whose default `~/.claude` the host expands. Until it is set there
//! is none, and nothing is read: the unit tests, and any test harness that
//! does not hand the plugin a folder, never reach the developer's own.

use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static FOLDER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Set (or clear, with an empty value) the folder.
pub fn set(value: &str) {
    let value = value.trim();
    FOLDER.with(|f| *f.borrow_mut() = (!value.is_empty()).then(|| PathBuf::from(value)));
}

/// The folder, if one is set.
pub fn get() -> Option<PathBuf> {
    FOLDER.with(|f| f.borrow().clone())
}
