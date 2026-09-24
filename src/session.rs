//! Child-process management for a streaming `claude` session.
//!
//! **Plain pipes** rather than a PTY: `stream-json` wants a clean
//! newline-delimited byte stream, not a terminal grid. Inside the sandbox the
//! host runs the CLI (`process`, which `plugin.json` asks for) and its reads
//! never block. Natively, for the unit tests, `std::process` with a reader
//! thread that blocks on `BufReader::read_line`. Either way the plugin's
//! methods only ever do non-blocking drains.

use std::path::PathBuf;

/// How to spawn the `claude` child process.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// The `claude` program: in the sandbox the name `plugin.json` lists
    /// (the host finds it on `PATH` or in `~/.local/bin`), natively a name or
    /// a path.
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

// ---------------------------------------------------------------------------
// In the sandbox: the host's pipes
// ---------------------------------------------------------------------------

/// A running `claude --output-format stream-json`. Dropping it drops the
/// host resource, which kills the program.
#[cfg(target_arch = "wasm32")]
pub struct Session {
    child: sicompass_pdk::process::Child,
    /// The unfinished line at the end of what has been read.
    partial: std::cell::RefCell<Vec<u8>>,
}

#[cfg(target_arch = "wasm32")]
impl Session {
    pub fn spawn(cfg: &SessionConfig) -> std::io::Result<Session> {
        let cwd = cfg.cwd.as_ref().map(|p| p.to_string_lossy().into_owned());
        let child = sicompass_pdk::process::Child::spawn(
            &cfg.program,
            &args(cfg),
            cwd.as_deref(),
            &[],
            &[],
            None,
        )
        .map_err(|e| {
            // The host says why; a program it cannot find reads as NotFound,
            // as a spawn outside the sandbox would.
            let kind = if e.contains("not found") {
                std::io::ErrorKind::NotFound
            } else {
                std::io::ErrorKind::Other
            };
            std::io::Error::new(kind, e)
        })?;
        Ok(Session {
            child,
            partial: std::cell::RefCell::new(Vec::new()),
        })
    }

    /// Take all complete JSONL lines that arrived since the last call.
    pub fn drain_lines(&self) -> Vec<String> {
        let mut partial = self.partial.borrow_mut();
        loop {
            let chunk = self.child.read(1 << 20);
            if chunk.is_empty() {
                break;
            }
            partial.extend(chunk);
        }
        let Some(end) = partial.iter().rposition(|b| *b == b'\n') else {
            return Vec::new();
        };
        let complete: Vec<u8> = partial.drain(..=end).collect();
        String::from_utf8_lossy(&complete)
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Take all stderr text that arrived so far.
    pub fn take_stderr(&self) -> String {
        let mut out = Vec::new();
        loop {
            let chunk = self.child.read_stderr(1 << 20);
            if chunk.is_empty() {
                break;
            }
            out.extend(chunk);
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Write one JSONL user message to the child's stdin.
    pub fn write_user(&mut self, json_line: &str) -> std::io::Result<()> {
        let mut bytes = json_line.as_bytes().to_vec();
        bytes.push(b'\n');
        self.child.write(&bytes).map_err(std::io::Error::other)
    }

    /// `true` until the child has exited and its output has all arrived.
    pub fn is_alive(&mut self) -> bool {
        self.child.try_wait().is_none()
    }
}

// ---------------------------------------------------------------------------
// Natively, for the tests: std::process
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
use std::io::{BufRead, BufReader, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

/// A spawned `claude --output-format stream-json` process.
///
/// `drain_lines()` is non-blocking and returns whatever complete JSONL lines
/// the background reader thread has buffered since the previous call.
#[cfg(not(target_arch = "wasm32"))]
pub struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Arc<Mutex<Vec<String>>>,
    stderr: Arc<Mutex<String>>,
    alive: Arc<AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Session {
    /// Spawn `cfg.program` in streaming-JSON mode with piped stdio.
    pub fn spawn(cfg: &SessionConfig) -> std::io::Result<Session> {
        let mut cmd = Command::new(&cfg.program);
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

    /// OS process id of the child. (Inside the sandbox the host reports it
    /// to the tab switcher itself.)
    #[cfg(test)]
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

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
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
        let mut child = Command::new("cat")
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
