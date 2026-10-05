//! Claude Code's own folder, `~/.claude`: every project's transcripts under
//! `projects/`, the personal skills, the enabled marketplace plugins.
//!
//! The folder is the `claudeFolder` setting, whose default `~/.claude` the app
//! expands (and so does [`set`], for a value that reaches it unexpanded).
//! Until it is set there is none, and nothing is read: the unit tests, and any
//! test harness that does not hand the plugin a folder, never reach the
//! developer's own.

use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static FOLDER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Set (or clear, with an empty value) the folder.
pub fn set(value: &str) {
    let value = value.trim();
    let folder = match value.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            crate::program::home_dir().map(|h| h.join(rest.trim_start_matches(['/', '\\'])))
        }
        _ => (!value.is_empty()).then(|| PathBuf::from(value)),
    };
    FOLDER.with(|f| *f.borrow_mut() = folder);
}

/// The folder, if one is set.
pub fn get() -> Option<PathBuf> {
    FOLDER.with(|f| f.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_is_the_home_folder_and_empty_is_none() {
        // Thread-local, so this cannot disturb another test's folder.
        set("~/.claude");
        assert_eq!(get(), crate::program::home_dir().map(|h| h.join(".claude")));
        set("/somewhere/else");
        assert_eq!(get(), Some(PathBuf::from("/somewhere/else")));
        set("~user/x");
        assert_eq!(
            get(),
            Some(PathBuf::from("~user/x")),
            "only `~` alone is home"
        );
        set("  ");
        assert_eq!(get(), None);
    }
}
