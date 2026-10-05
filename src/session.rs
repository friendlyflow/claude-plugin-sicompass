//! Child-process management for a streaming `claude` session.
//!
//! **Plain pipes** rather than a PTY: `stream-json` wants a clean
//! newline-delimited byte stream, not a terminal grid. `std::process`, with a
//! reader thread that blocks on `BufReader::read_line`, so the plugin's
//! methods only ever do non-blocking drains.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// How to spawn the `claude` child process.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// The `claude` program: a name, found as [`crate::program::resolve`]
    /// does (`PATH`, then `~/.local/bin`, then macOS's application bundles),
    /// or a path.
    pub program: String,
    /// `--permission-mode` value: default | acceptEdits | plan | bypassPermissions.
    pub permission_mode: String,
    /// `--model` override; `None` omits the flag.
    pub model: Option<String>,
    /// Free-form extra CLI args appended verbatim.
    pub extra_args: Vec<String>,
    /// Working directory for the child; `None` inherits the parent's.
    pub cwd: Option<PathBuf>,
    /// `--resume <session_id>` — set when re-spawning after an unexpected exit.
    pub resume: Option<String>,
    /// Add `--include-partial-messages` to stream token-level deltas.
    pub include_partial: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            program: "claude".to_owned(),
            permission_mode: "default".to_owned(),
            model: None,
            extra_args: Vec::new(),
            cwd: None,
            resume: None,
            include_partial: true,
        }
    }
}

/// The command line for `cfg`, after the program.
fn args(cfg: &SessionConfig) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
    ]
    .map(str::to_owned)
    .to_vec();
    if cfg.include_partial {
        args.push("--include-partial-messages".to_owned());
    }
    args.push("--permission-mode".to_owned());
    args.push(cfg.permission_mode.clone());
    if let Some(model) = cfg.model.as_ref().filter(|m| !m.is_empty()) {
        args.push("--model".to_owned());
        args.push(model.clone());
    }
    if let Some(resume) = cfg.resume.as_ref().filter(|r| !r.is_empty()) {
        args.push("--resume".to_owned());
        args.push(resume.clone());
    }
    args.extend(cfg.extra_args.iter().cloned());
    args
}

/// A spawned `claude --output-format stream-json` process.
///
/// `drain_lines()` is non-blocking and returns whatever complete JSONL lines
/// the background reader thread has buffered since the previous call.
pub struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Arc<Mutex<Vec<String>>>,
    stderr: Arc<Mutex<String>>,
    alive: Arc<AtomicBool>,
}

impl Session {
    /// Spawn `cfg.program` in streaming-JSON mode with piped stdio.
    pub fn spawn(cfg: &SessionConfig) -> std::io::Result<Session> {
        // Not found anywhere: the bare name, which fails with `NotFound`.
        let exe =
            crate::program::resolve(&cfg.program).unwrap_or_else(|| PathBuf::from(&cfg.program));
        let mut cmd = sicompass_sdk::plugin::command(exe);
        cmd.args(args(cfg));
        if let Some(cwd) = &cfg.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let stderr_buf = Arc::new(Mutex::new(String::new()));
        let alive = Arc::new(AtomicBool::new(true));

        // stdout reader: BufReader::read_line reassembles partial lines across
        // read boundaries for free, so each pushed entry is a complete line.
        {
            let lines = Arc::clone(&lines);
            let alive = Arc::clone(&alive);
            thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut buf = String::new();
                loop {
                    buf.clear();
                    match reader.read_line(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = buf.trim_end_matches(['\r', '\n']).to_owned();
                            if !line.is_empty()
                                && let Ok(mut l) = lines.lock()
                            {
                                l.push(line);
                            }
                        }
                    }
                }
                alive.store(false, Ordering::SeqCst);
            });
        }

        // stderr reader: accumulate so the provider can surface a real error.
        {
            let stderr_buf = Arc::clone(&stderr_buf);
            thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut buf = String::new();
                loop {
                    buf.clear();
                    match reader.read_line(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if let Ok(mut s) = stderr_buf.lock() {
                                s.push_str(&buf);
                            }
                        }
                    }
                }
            });
        }

        Ok(Session {
            child,
            stdin,
            lines,
            stderr: stderr_buf,
            alive,
        })
    }

    /// Take all complete JSONL lines buffered since the last call.
    pub fn drain_lines(&self) -> Vec<String> {
        match self.lines.lock() {
            Ok(mut l) => std::mem::take(&mut *l),
            Err(_) => Vec::new(),
        }
    }

    /// Take all stderr text accumulated so far.
    pub fn take_stderr(&self) -> String {
        match self.stderr.lock() {
            Ok(mut s) => std::mem::take(&mut *s),
            Err(_) => String::new(),
        }
    }

    /// Write one JSONL user message to the child's stdin.
    pub fn write_user(&mut self, json_line: &str) -> std::io::Result<()> {
        self.stdin.write_all(json_line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()
    }

    /// `true` while the stdout reader has not yet seen EOF and the child has
    /// not been reaped.
    pub fn is_alive(&mut self) -> bool {
        if !self.alive.load(Ordering::SeqCst) {
            return false;
        }
        !matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// OS process id of the child, which the app's tab switcher names the
    /// tab after.
    pub fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }

    /// Kill the child and reap it.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.alive.store(false, Ordering::SeqCst);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_missing_binary_errors() {
        let cfg = SessionConfig {
            program: "definitely-not-claude-xyz-9000".to_owned(),
            ..SessionConfig::default()
        };
        let result = Session::spawn(&cfg);
        assert!(result.is_err(), "missing binary should fail");
        assert_eq!(result.err().unwrap().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn config_default_is_plain_claude() {
        let cfg = SessionConfig::default();
        assert_eq!(cfg.program, "claude");
        assert_eq!(cfg.permission_mode, "default");
        assert!(cfg.model.is_none());
        assert!(cfg.resume.is_none());
    }

    // Reader-thread plumbing: a child that echoes stdin to stdout should have
    // each written line surface back through `drain_lines()`. `cat` is the
    // simplest such program; skipped on platforms without it. The reader/
    // stderr threads are wired exactly as in `Session::spawn` — only the
    // process and its args differ (plain `cat`, no claude flags).
    #[test]
    #[cfg(unix)]
    fn write_then_drain_round_trips_a_line() {
        let mut child = std::process::Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn cat");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let alive = Arc::new(AtomicBool::new(true));
        {
            let lines = Arc::clone(&lines);
            let alive = Arc::clone(&alive);
            thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut buf = String::new();
                loop {
                    buf.clear();
                    match reader.read_line(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = buf.trim_end_matches(['\r', '\n']).to_owned();
                            if !line.is_empty() {
                                lines.lock().unwrap().push(line);
                            }
                        }
                    }
                }
                alive.store(false, Ordering::SeqCst);
            });
        }
        let mut session = Session {
            child,
            stdin,
            lines,
            stderr: Arc::new(Mutex::new(String::new())),
            alive,
        };
        session.write_user("hello").expect("write");
        let mut drained = Vec::new();
        for _ in 0..200 {
            drained = session.drain_lines();
            if !drained.is_empty() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(drained, vec!["hello".to_owned()]);
        session.kill();
    }

    #[test]
    fn the_command_line_carries_only_what_is_set() {
        let mut cfg = SessionConfig::default();
        let plain = args(&cfg);
        assert!(plain.contains(&"--include-partial-messages".to_owned()));
        assert!(!plain.contains(&"--model".to_owned()));
        assert!(!plain.contains(&"--resume".to_owned()));
        cfg.model = Some("opus".to_owned());
        cfg.resume = Some("abc".to_owned());
        cfg.include_partial = false;
        cfg.extra_args = vec!["--x".to_owned()];
        let a = args(&cfg);
        assert!(a.windows(2).any(|w| w == ["--model", "opus"]));
        assert!(a.windows(2).any(|w| w == ["--resume", "abc"]));
        assert!(!a.contains(&"--include-partial-messages".to_owned()));
        assert_eq!(a.last().map(String::as_str), Some("--x"));
    }
}
