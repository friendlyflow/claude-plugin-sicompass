//! Small files that let a new plugin process skip work an earlier one did.
//!
//! The app starts one plugin process per tab, and starts them again after an
//! update, a restart or a reboot. What a process learned in memory (the session
//! list's per-transcript metadata, Claude Code's slash commands read out of a
//! ~300 MB CLI) used to die with it, so the first session list and the first
//! skills list of every process paid for a full scan, from a cold disk more
//! often than not. These files carry it over.
//!
//! They live in `cache/` in the plugin's own folder ([`storage_dir`], which
//! `"storage": true` in `plugin.json` asks for and which the approval does not
//! cover, since a folder of its own grants nothing). Everything there can be
//! rebuilt: a missing, unreadable or outdated file only means the scan runs
//! again. Outside sicompass there is no such folder, and nothing is cached.
//!
//! [`storage_dir`]: sicompass_sdk::plugin::storage_dir
//! Each file records a `format`, and a reader that expects another one ignores
//! it, so a change to what is stored bumps the caller's constant and costs one
//! re-scan.
//!
//! Several processes may write the same file (two claude tabs). Each write goes
//! to a file of its own and is renamed over the old one, so a reader sees one
//! whole file or the other, and the last writer wins. Losing the other's entries
//! costs nothing but a re-read of those transcripts later.
//!
//! Unit tests never touch the real cache: under `cfg(test)` there is no
//! directory unless a test injects one with [`_set_test_cache_dir`], which is
//! thread-local and so affects no other test. The app's integration tests grant
//! this plugin no storage folder, so they cache nothing.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static TEST_CACHE_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Point this thread's cache at `path` instead of the real one. Test-only.
#[doc(hidden)]
pub fn _set_test_cache_dir(path: Option<PathBuf>) {
    TEST_CACHE_DIR.with(|c| *c.borrow_mut() = path);
}

fn dir() -> Option<PathBuf> {
    if let Some(d) = TEST_CACHE_DIR.with(|c| c.borrow().clone()) {
        return Some(d);
    }
    if cfg!(test) {
        return None;
    }
    sicompass_sdk::plugin::storage_dir().map(|d| d.join("cache"))
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    format: u32,
    data: T,
}

/// The data in `name`, when the file is there, parses, and has this `format`.
pub(crate) fn load<T: DeserializeOwned>(name: &str, format: u32) -> Option<T> {
    let bytes = std::fs::read(dir()?.join(name)).ok()?;
    let envelope: Envelope<T> = serde_json::from_slice(&bytes).ok()?;
    (envelope.format == format).then_some(envelope.data)
}

/// Replace `name` with `data`. Failing is silent: the cache is an optimisation.
pub(crate) fn store<T: Serialize>(name: &str, format: u32, data: &T) {
    let Some(dir) = dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(&Envelope { format, data }) else {
        return;
    };
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, dir.join(name)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_read_or_written_without_a_directory() {
        _set_test_cache_dir(None);
        store("t.json", 1, &vec![1, 2, 3]);
        assert_eq!(load::<Vec<i32>>("t.json", 1), None);
    }

    #[test]
    fn a_stored_value_loads_back_with_its_format_only() {
        let tmp = tempfile::tempdir().unwrap();
        _set_test_cache_dir(Some(tmp.path().join("nested")));
        store("t.json", 3, &vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(
            load::<Vec<String>>("t.json", 3),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(load::<Vec<String>>("t.json", 4), None, "another format");
        assert_eq!(load::<Vec<String>>("other.json", 3), None, "another file");
        _set_test_cache_dir(None);
    }

    #[test]
    fn a_damaged_file_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        _set_test_cache_dir(Some(tmp.path().to_owned()));
        std::fs::write(tmp.path().join("t.json"), b"{\"format\":1,\"da").unwrap();
        assert_eq!(load::<Vec<i32>>("t.json", 1), None);
        _set_test_cache_dir(None);
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        _set_test_cache_dir(Some(tmp.path().to_owned()));
        store("t.json", 1, &1);
        store("t.json", 1, &2);
        let names: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["t.json"]);
        assert_eq!(load::<i32>("t.json", 1), Some(2));
        _set_test_cache_dir(None);
    }
}
