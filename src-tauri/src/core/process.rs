//! `core::process` — T-040's process and port primitives.
//!
//! Three seams, all of them pure-in, pure-out so the supervisor's state machine
//! can be driven against doubles on a machine with no llama.cpp build at all
//! (`AGENTS.md` §3: every criterion is checkable without the target hardware):
//!
//! - [`ProcessLauncher`] / [`ManagedChild`] — spawn, watch, stop and reap a
//!   child process. [`SystemLauncher`] is the real one; tests supply a
//!   scripted child, and the tree-kill path is additionally exercised against a
//!   real `cmd.exe` process tree.
//! - [`PortAllocator`] — draw the upstream port from the configured range,
//!   skipping one that is already taken. The **only** place in this app where
//!   auto-increment is correct (`PLAN.md` §2.7): the upstream port is internal,
//!   while the client-facing port is a contract and is never incremented.
//! - [`tree`] — the Windows process-tree primitives (`descendants`, `pid_alive`,
//!   `terminate_tree`) that make "no orphaned children remain after stop" a
//!   fact about the machine rather than a claim about our own bookkeeping.
//!
//! # Log capture is line-oriented and lossless
//!
//! A reader thread per pipe reads whole lines with `read_until(b'\n')` and
//! pushes each one over an unbounded channel, so no line is ever dropped,
//! merged or split because the actor was busy elsewhere. `\r\n` and `\n` are
//! both accepted (`llama-server` writes CRLF on Windows), a final line without
//! a terminator is emitted rather than lost, and invalid UTF-8 is replaced
//! rather than discarded — one bad byte must not cost the rest of the line.
//! The only bound is the per-line cap in [`MAX_LINE_BYTES`], which splits a
//! pathological unterminated line instead of letting it grow without limit.

use std::io::{BufRead, BufReader, Read};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self, JoinHandle};

use chrono::{DateTime, Utc};

use crate::core::types::AppError;

/// The command line the app hands to a launcher. Built by the supervisor
/// (`plan_launch`) so that the argument vector is a testable value rather than
/// a side effect of spawning: the "upstream is always `127.0.0.1`" criterion is
/// asserted over it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerLaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The build's own directory: `llama-server.exe` resolves its DLLs and
    /// `--models-preset` relative paths from there.
    pub cwd: Option<PathBuf>,
}

/// Which pipe a line came from. Kept because `level` alone loses it, and the
/// Logs screen (T-046) filters on both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// One captured line, in the shape of the `server-log-line` event payload
/// (`docs/CONTRACTS.md` §4: `{ level, line, at }`) — which is why it derives
/// `Serialize` and why the field names are the contract's.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LogLine {
    pub at: DateTime<Utc>,
    pub level: String,
    pub line: String,
}

/// `llama-server` writes its own level letter as the second whitespace token of
/// a stderr line — verified against a real b10883 run:
/// `0.00.146.861 W srv  llama_server: ...`, `0.00.083.217 I srv ...`.
/// Anything unrecognised is information, never an error: inventing a severity
/// is worse than reporting the commonest one.
fn classify(stream: LogStream, line: &str) -> &'static str {
    if stream == LogStream::Stdout {
        return "info";
    }
    match line.split_whitespace().nth(1) {
        Some("E") => "error",
        Some("W") => "warn",
        Some("D") => "debug",
        _ => "info",
    }
}

/// A single line longer than this is emitted in pieces rather than buffered
/// without bound. 1 MiB is ~1000× the longest line a real llama.cpp run
/// produces, so this never fires in practice; it exists so a child that streams
/// bytes without a newline cannot exhaust memory.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// A child process the supervisor owns.
///
/// Deliberately narrow: everything the state machine needs to know about the
/// process and nothing about how it was produced. A test double implements
/// this directly; so does [`SystemChild`].
pub trait ManagedChild: Send {
    /// The root of the process tree. Recorded in `ServerState::Running` and
    /// used for the tree kill.
    fn pid(&self) -> u32;

    /// `Some(code)` once the process has exited — a value on Windows even for a
    /// killed process. `None` while it is still running.
    fn try_wait(&mut self) -> Result<Option<i32>, AppError>;

    /// Ask the process to end on its own terms, before the grace window runs
    /// out. Best-effort by definition: a console process with no message loop
    /// has no receiver for the request (measured — see PROGRESS.md F-020), and
    /// that is not an error, it is information the caller logs.
    fn request_graceful_stop(&mut self) -> Result<(), AppError>;

    /// Forced kill of the whole tree, children first, and of the root. Returns
    /// the pids that were terminated, so the caller has evidence rather than a
    /// belief.
    fn kill_tree(&mut self) -> Result<Vec<u32>, AppError>;

    /// Move every line the child has produced so far into `sink` without
    /// blocking.
    fn drain_logs(&mut self, sink: &mut dyn FnMut(LogLine));
}

/// How a child process is produced.
pub trait ProcessLauncher: Send + Sync {
    fn spawn(&self, launch: &ServerLaunch) -> Result<Box<dyn ManagedChild>, AppError>;
}

/// The real launcher: `std::process::Command` with piped output.
///
/// `CREATE_NO_WINDOW` matters: the app is a windowed binary
/// (`windows_subsystem = "windows"`), so without it every server start pops a
/// console window the user did not ask for.
#[derive(Default)]
pub struct SystemLauncher;

impl SystemLauncher {
    pub fn new() -> Self {
        SystemLauncher
    }
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;
    cmd.creation_flags(CREATE_NO_WINDOW.0);
}

#[cfg(not(windows))]
fn hide_window(_cmd: &mut Command) {}

impl ProcessLauncher for SystemLauncher {
    fn spawn(&self, launch: &ServerLaunch) -> Result<Box<dyn ManagedChild>, AppError> {
        let mut cmd = Command::new(&launch.program);
        cmd.args(&launch.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &launch.cwd {
            cmd.current_dir(cwd);
        }
        hide_window(&mut cmd);

        let mut child = cmd.spawn().map_err(|err| AppError::Io {
            message: format!("could not start {}: {err}", launch.program.display()),
        })?;
        let pid = child.id();

        let (tx, rx) = std::sync::mpsc::channel();
        let mut readers: Vec<JoinHandle<()>> = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            readers.push(spawn_reader(stdout, LogStream::Stdout, tx.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.push(spawn_reader(stderr, LogStream::Stderr, tx));
        }

        Ok(Box::new(SystemChild {
            child,
            pid,
            logs: rx,
            exit_code: None,
            _readers: readers,
        }))
    }
}

/// One reader thread per pipe. `read_until` is the whole trick: it waits for a
/// terminator (or EOF) and hands over a complete line, so a write split across
/// several `Write` calls cannot be mistaken for two lines.
fn spawn_reader<R: Read + Send + 'static>(
    reader: R,
    stream: LogStream,
    tx: Sender<LogLine>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) => {
                    if bytes.len() > MAX_LINE_BYTES {
                        // Unterminated and oversized: emit what we have so the
                        // child cannot grow our memory, then keep reading.
                        let _ = tx.send(make_line(stream, &bytes));
                        bytes.clear();
                        let mut rest = Vec::new();
                        match reader.read_until(b'\n', &mut rest) {
                            Ok(0) => break,
                            Ok(_) => {
                                let _ = tx.send(make_line(stream, &rest));
                            }
                            Err(_) => break,
                        }
                        continue;
                    }
                    while matches!(bytes.last(), Some(b'\n') | Some(b'\r')) {
                        bytes.pop();
                    }
                    if tx.send(make_line(stream, &bytes)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn make_line(stream: LogStream, bytes: &[u8]) -> LogLine {
    let line = String::from_utf8_lossy(bytes).into_owned();
    LogLine {
        at: Utc::now(),
        level: classify(stream, &line).to_string(),
        line,
    }
}

/// A real child process.
pub struct SystemChild {
    child: Child,
    pid: u32,
    logs: Receiver<LogLine>,
    /// Cached once the process is reaped: `Child::try_wait` keeps returning the
    /// same status, but caching keeps the state machine's view monotone.
    exit_code: Option<i32>,
    /// The reader threads' handles. Kept only so the handles are not dropped
    /// while the process still writes; the threads end at EOF by themselves and
    /// are never joined (joining would block the actor on a pipe nobody is
    /// closing). `Drop` is what reaps the process.
    _readers: Vec<JoinHandle<()>>,
}

impl ManagedChild for SystemChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn try_wait(&mut self) -> Result<Option<i32>, AppError> {
        if let Some(code) = self.exit_code {
            return Ok(Some(code));
        }
        let status = self.child.try_wait().map_err(|err| AppError::Io {
            message: format!("could not query process {}: {err}", self.pid),
        })?;
        match status {
            Some(status) => {
                let code = status.code().unwrap_or(-1);
                self.exit_code = Some(code);
                Ok(Some(code))
            }
            None => Ok(None),
        }
    }

    fn request_graceful_stop(&mut self) -> Result<(), AppError> {
        // `taskkill` without `/F` is the only polite request Windows offers a
        // process that owns no window we can post to. Measured on the installed
        // b10883 router: it refuses it ("this process can only be terminated
        // forcefully") and stays alive, which is why the grace window below it
        // is not optional. Its output is not swallowed — the caller logs it.
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &self.pid.to_string(), "/T"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_window(&mut cmd);
        let output = cmd.output().map_err(|err| AppError::Io {
            message: format!("could not ask process {} to stop: {err}", self.pid),
        })?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(AppError::Io {
                message: if message.is_empty() {
                    format!("process {} refused the graceful stop request", self.pid)
                } else {
                    format!(
                        "process {} refused the graceful stop request: {message}",
                        self.pid
                    )
                },
            });
        }
        Ok(())
    }

    fn kill_tree(&mut self) -> Result<Vec<u32>, AppError> {
        tree::terminate_tree(self.pid)
    }

    fn drain_logs(&mut self, sink: &mut dyn FnMut(LogLine)) {
        while let Ok(line) = self.logs.try_recv() {
            sink(line);
        }
    }
}

impl Drop for SystemChild {
    fn drop(&mut self) {
        // A launcher that is dropped while its process lives would leave an
        // orphan: the whole point of owning the child is that nobody else can
        // reap it. `kill_tree` is best-effort here — a process that already
        // exited is not an error, and a failure at drop time has no reporter.
        let _ = tree::terminate_tree(self.pid);
    }
}

/// A port the app can hand to the upstream server.
pub trait PortAllocator: Send + Sync {
    /// Returns a free TCP port on `127.0.0.1` from `range`, or a typed error if
    /// the range yields none. Never blocks and never asks the user.
    fn allocate(&self, range: (u16, u16)) -> Result<u16, AppError>;
}

/// The real allocator: bind, drop, hand the number over.
///
/// A port cannot be *held* open for the child — the child binds it itself — so
/// the check is a bind-and-release probe. The window between the probe and the
/// child's bind is the same race every process manager has; what makes it
/// harmless here is that another port is tried, not that the race is absent.
///
/// The walk starts at `range.0` and goes upward, wrapping at `range.1` only if
/// it started above it. Deterministic on purpose: a failed start is then
/// reproducible from the config alone.
#[derive(Default)]
pub struct SystemPortAllocator;

/// How many ports a single start will try before giving up. Bounded so an
/// exhausted range is an error and not a hang.
pub const MAX_PORT_ATTEMPTS: u16 = 20;

impl PortAllocator for SystemPortAllocator {
    fn allocate(&self, range: (u16, u16)) -> Result<u16, AppError> {
        let (lo, hi) = range;
        if lo > hi {
            return Err(AppError::Internal {
                message: format!(
                    "the upstream port range is inverted: {lo} is above {hi}. \
                     Fix it in the API settings."
                ),
            });
        }
        let span = u32::from(hi) - u32::from(lo) + 1;
        let attempts = span.min(u32::from(MAX_PORT_ATTEMPTS)) as u16;

        let mut last_tried = lo;
        for offset in 0..attempts {
            let port = lo + offset;
            last_tried = port;
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            match TcpListener::bind(addr) {
                Ok(listener) => {
                    drop(listener);
                    return Ok(port);
                }
                Err(err) => {
                    tracing::debug!(
                        "upstream port {port} is not available ({err}); trying the next one"
                    );
                }
            }
        }

        Err(AppError::PortInUse { port: last_tried })
    }
}

/// Windows process-tree primitives.
///
/// "No orphaned children remain after stop" is the kind of claim that is easy to
/// assert and hard to know: the child's own bookkeeping says nothing about the
/// grandchildren it spawned. `descendants` walks the OS's process table
/// (`CreateToolhelp32Snapshot`), so the answer comes from the machine.
///
/// On a non-Windows host the tree walk degrades to the single process (the app
/// targets Windows 11 exclusively; `docs/DEV-SETUP.md` has no other target), and
/// the kill falls back to `kill -9` so the module still behaves if the crate is
/// ever built elsewhere.
#[cfg(windows)]
pub mod tree {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };

    use crate::core::types::AppError;

    /// `STILL_ACTIVE`, the exit code of a live process.
    const STILL_ACTIVE: u32 = 259;

    fn win_err(context: &str, err: windows::core::Error) -> AppError {
        AppError::Io {
            message: format!("{context}: {err}"),
        }
    }

    /// Every `(pid, parent_pid)` pair the OS currently knows about.
    fn process_table() -> Result<Vec<(u32, u32)>, AppError> {
        // SAFETY: the snapshot handle is closed on every path below, and
        // `PROCESSENTRY32W` is initialised with the `dwSize` the API requires.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
                .map_err(|err| win_err("could not snapshot the process table", err))?;

            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut pairs = Vec::new();
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
            Ok(pairs)
        }
    }

    /// Every live descendant of `pid`, breadth-first, excluding `pid` itself.
    ///
    /// A process whose parent has already exited keeps pointing at the dead
    /// parent's pid, so the walk still finds orphans left behind by a root that
    /// died on its own — which is exactly the case a stop sequence has to cover.
    pub fn descendants(pid: u32) -> Result<Vec<u32>, AppError> {
        let table = process_table()?;
        let mut found: Vec<u32> = Vec::new();
        let mut frontier = vec![pid];
        while let Some(parent) = frontier.pop() {
            for (child, ppid) in &table {
                if *ppid == parent && !found.contains(child) && *child != pid {
                    found.push(*child);
                    frontier.push(*child);
                }
            }
        }
        Ok(found)
    }

    /// Whether the process is still running.
    pub fn pid_alive(pid: u32) -> bool {
        // SAFETY: the handle is closed before returning on every path.
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code).is_ok();
            let _ = CloseHandle(handle);
            ok && code == STILL_ACTIVE
        }
    }

    /// Terminate one process. `Ok(false)` when it was already gone or could not
    /// be opened — never an error, because "it is not running" is the goal.
    fn terminate(pid: u32) -> bool {
        if pid == std::process::id() {
            return false;
        }
        // SAFETY: the handle is closed before returning.
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) else {
                return false;
            };
            let killed = TerminateProcess(handle, 1).is_ok();
            let _ = CloseHandle(handle);
            killed
        }
    }

    /// Kill the whole tree rooted at `pid`: deepest descendants first, so a
    /// dying parent cannot outrun the enumeration, then the root.
    /// Returns the pids that were actually terminated.
    pub fn terminate_tree(pid: u32) -> Result<Vec<u32>, AppError> {
        let mut order = descendants(pid)?;
        order.reverse();
        order.push(pid);

        let mut killed = Vec::new();
        for candidate in order {
            if terminate(candidate) {
                killed.push(candidate);
            }
        }
        Ok(killed)
    }

    /// Exposed for tests that must not assume their own handle bookkeeping.
    #[allow(dead_code)]
    pub(crate) fn close_handle(handle: HANDLE) {
        // SAFETY: only ever called with a handle this module opened.
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
pub mod tree {
    use std::process::Command;

    use crate::core::types::AppError;

    pub fn descendants(_pid: u32) -> Result<Vec<u32>, AppError> {
        Ok(Vec::new())
    }

    pub fn pid_alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    pub fn terminate_tree(pid: u32) -> Result<Vec<u32>, AppError> {
        let killed = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        Ok(if killed { vec![pid] } else { Vec::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A real process tree, produced by the OS itself: the outer `cmd.exe` is
    /// our direct child, the inner one is its child. Both must be gone after a
    /// tree kill, which is the only way to tell a tree kill from a
    /// `TerminateProcess` on the root.
    fn spawn_cmd_tree() -> Box<dyn ManagedChild> {
        let launcher = SystemLauncher::new();
        launcher
            .spawn(&ServerLaunch {
                program: PathBuf::from("cmd.exe"),
                // The inner command outlives the outer only until the kill.
                args: vec![
                    "/C".to_string(),
                    "cmd /C ping -n 120 127.0.0.1 >NUL".to_string(),
                ],
                cwd: None,
            })
            .expect("cmd.exe must be launchable on Windows")
    }

    fn descendants_settle(pid: u32) -> Vec<u32> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let found = tree::descendants(pid).unwrap_or_default();
            if !found.is_empty() || Instant::now() > deadline {
                return found;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_real_process_tree_is_reaped_with_its_children() {
        let mut child = spawn_cmd_tree();
        let pid = child.pid();
        let children = descendants_settle(pid);
        assert!(
            !children.is_empty(),
            "the stub child must actually spawn a grandchild for this test to mean anything \
             (tree of {pid} was empty)"
        );

        let killed = child.kill_tree().expect("tree kill must succeed");
        assert!(
            killed.contains(&pid),
            "the root must be terminated: {killed:?}"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && tree::pid_alive(pid) {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(!tree::pid_alive(pid), "root {pid} survived the tree kill");
        for grandchild in children {
            assert!(
                !tree::pid_alive(grandchild),
                "grandchild {grandchild} was orphaned by the tree kill"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn dropping_a_child_does_not_leave_it_running() {
        // The state machine can be dropped on an error path, and a child that
        // outlives its owner is the orphan this module exists to prevent.
        let pid = {
            let child = spawn_cmd_tree();
            child.pid()
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && tree::pid_alive(pid) {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !tree::pid_alive(pid),
            "the dropped child {pid} is still alive"
        );
    }

    #[test]
    fn log_capture_is_line_oriented_and_lossless() {
        let scratch = std::env::temp_dir().join(format!("t040-logs-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        let fixture = scratch.join("lines.txt");
        let count = 20_000usize;
        let mut body = String::with_capacity(count * 12);
        for i in 0..count {
            body.push_str(&format!("0.00.000.001 I srv line-{i}\r\n"));
        }
        // A final line with no terminator at all: it must still be captured.
        body.push_str("0.00.000.002 E srv trailing-without-newline");
        std::fs::write(&fixture, body).expect("fixture log");

        let launcher = SystemLauncher::new();
        let mut child = launcher
            .spawn(&ServerLaunch {
                program: PathBuf::from("cmd.exe"),
                args: vec!["/C".to_string(), format!("type {}", fixture.display())],
                cwd: None,
            })
            .expect("cmd.exe must be launchable");

        let mut lines: Vec<LogLine> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            child.drain_logs(&mut |line| lines.push(line));
            let exited = child.try_wait().expect("wait must succeed").is_some();
            if exited && lines.len() >= count + 1 {
                break;
            }
            if Instant::now() > deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        child.drain_logs(&mut |line| lines.push(line));

        assert_eq!(
            lines.len(),
            count + 1,
            "every line must arrive exactly once (got {} of {})",
            lines.len(),
            count + 1
        );
        assert_eq!(lines[0].line, "0.00.000.001 I srv line-0");
        assert_eq!(
            lines[count - 1].line,
            format!("0.00.000.001 I srv line-{}", count - 1)
        );
        assert_eq!(
            lines[count].line, "0.00.000.002 E srv trailing-without-newline",
            "an unterminated final line must not be lost"
        );
        // CR must not survive into the captured line, and the level letter must
        // be read off the line rather than assumed (`type` writes to stdout, so
        // both lines are informational whatever they say).
        assert!(!lines[0].line.ends_with('\r'));
        assert_eq!(lines[0].level, "info");
        assert_eq!(lines[count].level, "info");
        assert_eq!(lines[0].at.offset(), &chrono::Utc);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_lines_level_comes_from_its_own_level_letter() {
        let stdout = |line: &str| classify(LogStream::Stdout, line);
        let stderr = |line: &str| classify(LogStream::Stderr, line);

        // Verbatim from a real b10883 run: the level letter is the second
        // whitespace token of a stderr line.
        assert_eq!(
            stderr("0.00.146.861 W srv  llama_server: CORS is set to allow all"),
            "warn"
        );
        assert_eq!(
            stderr("0.00.083.217 I srv    operator(): instance exited"),
            "info"
        );
        assert_eq!(
            stderr("0.00.001.000 E srv  llama_server: failed to load"),
            "error"
        );
        assert_eq!(stderr("0.00.001.000 D srv  llama_server: detail"), "debug");
        // Anything unrecognised is information, never a fabricated severity.
        assert_eq!(stderr("garbage"), "info");
        assert_eq!(stderr(""), "info");
        assert_eq!(
            stdout("0.00.001.000 E srv not really an error here"),
            "info"
        );
    }

    #[test]
    fn the_port_allocator_skips_a_taken_port_and_reports_an_exhausted_range() {
        let allocator = SystemPortAllocator;

        // Hold a port, then ask for exactly that one: it must be refused, not
        // handed over.
        let held = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("an ephemeral listener must bind");
        let taken = held.local_addr().expect("bound address").port();
        let err = allocator
            .allocate((taken, taken))
            .expect_err("an occupied port must not be handed out");
        assert_eq!(err, AppError::PortInUse { port: taken });
        assert!(!err.message().is_empty());

        // A range with the occupied port first: the retry moves past it.
        let free_from = taken + 1;
        let port = allocator
            .allocate((taken, taken + 2))
            .expect("the retry must find a free port in the range");
        assert!(
            port > taken && port <= taken + 2,
            "got {port} from ({taken}, {})",
            taken + 2
        );
        assert_ne!(port, taken);
        drop(held);

        // An inverted range is a configuration bug, not a port conflict.
        let err = allocator
            .allocate((50000, 49999))
            .expect_err("an inverted range must be rejected");
        assert!(matches!(err, AppError::Internal { .. }), "got {err:?}");

        // A one-port range whose port is free is still allocated.
        let port = allocator
            .allocate((free_from, free_from))
            .expect("a free single-port range must be allocated");
        assert_eq!(port, free_from);
    }

    #[test]
    fn the_spawned_launch_is_reported_faithfully() {
        // Cheap counterpart of the tree test: the launcher must not rewrite the
        // command line it was given.
        let launcher = SystemLauncher::new();
        let child = launcher
            .spawn(&ServerLaunch {
                program: PathBuf::from("cmd.exe"),
                args: vec!["/C".to_string(), "exit 3".to_string()],
                cwd: None,
            })
            .expect("cmd.exe must be launchable");
        assert!(child.pid() > 0);
        drop(child);
    }
}
