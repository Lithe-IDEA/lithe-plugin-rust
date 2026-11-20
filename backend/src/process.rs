//! Runs Cargo and rustup. A child gets its own process group (a job of `taskkill /T`
//! on Windows), so cancelling a request stops Cargo together with the
//! compilers and build scripts it started. Its output comes back in
//! chunks as it arrives.

use lithe_plugin_sdk::{Cancellation, FunctionError, PluginError};
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// How often a waiting request looks at its cancellation.
const POLL: Duration = Duration::from_millis(50);
/// How long output may keep arriving after Cargo exited, from a process it
/// left behind that still holds the pipe. The rest is not waited for.
const DRAIN: Duration = Duration::from_secs(2);

/// One program to run.
#[derive(Debug, Clone)]
pub struct Invocation {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub directory: PathBuf,
    /// Variables set over the backend's own environment.
    pub environment: Vec<(OsString, OsString)>,
}

/// Which pipe a chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pipe {
    Stdout,
    Stderr,
}

enum Message {
    Chunk(Pipe, Vec<u8>),
    Closed,
}

/// Runs `invocation` to its end and answers its exit code. `chunk` sees the
/// output as it arrives; an error from it stops the program and is the
/// answer. A cancelled request stops the program and answers `Cancelled`.
pub fn run(
    invocation: &Invocation,
    cancellation: &Cancellation,
    mut chunk: impl FnMut(Pipe, &[u8]) -> Result<(), FunctionError>,
) -> Result<i32, FunctionError> {
    cancellation.check()?;
    let mut command = Command::new(&invocation.program);
    command
        .args(&invocation.arguments)
        .current_dir(&invocation.directory)
        .envs(
            invocation
                .environment
                .iter()
                .map(|(key, value)| (key, value)),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate(&mut command);
    let mut child = command.spawn().map_err(|error| {
        let name = invocation
            .program
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        PluginError::new(
            "rust.startFailed",
            format!("{name} could not start: {}.", error.kind()),
        )
    })?;

    let (sender, receiver) = mpsc::channel();
    let mut open = 0;
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, Pipe::Stdout, sender.clone());
        open += 1;
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, Pipe::Stderr, sender.clone());
        open += 1;
    }
    drop(sender);

    let mut exited: Option<(ExitStatus, Instant)> = None;
    loop {
        if cancellation.is_cancelled() {
            stop(&mut child, exited.is_some());
            return Err(FunctionError::Cancelled);
        }
        if open == 0 {
            break;
        }
        if let Some((_, at)) = exited {
            if at.elapsed() > DRAIN {
                break;
            }
        }
        match receiver.recv_timeout(POLL) {
            Ok(Message::Chunk(pipe, bytes)) => {
                if let Err(error) = chunk(pipe, &bytes) {
                    stop(&mut child, exited.is_some());
                    return Err(error);
                }
            }
            Ok(Message::Closed) => open -= 1,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => open = 0,
        }
        if exited.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                exited = Some((status, Instant::now()));
            }
        }
    }
    let status = match exited {
        Some((status, _)) => status,
        None => loop {
            if let Some(status) = child.try_wait().map_err(FunctionError::from)? {
                break status;
            }
            if cancellation.wait_timeout(POLL) {
                stop(&mut child, false);
                return Err(FunctionError::Cancelled);
            }
        },
    };
    // A process Cargo left running on purpose, such as a compiler cache
    // server, keeps running.
    Ok(code(status))
}

/// Reads `source` on a thread of its own until it closes.
fn pump(mut source: impl Read + Send + 'static, pipe: Pipe, sender: mpsc::Sender<Message>) {
    thread::spawn(move || {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if sender
                        .send(Message::Chunk(pipe, buffer[..read].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
        let _ = sender.send(Message::Closed);
    });
}

/// The exit code, or 128 plus the signal that ended the program on Unix.
fn code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

#[cfg(unix)]
fn isolate(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn isolate(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Stops the child and everything it started, then reaps it. Once the
/// child was reaped its ID may name another process, so only the pipes'
/// remaining writers are left to end on their own.
fn stop(child: &mut Child, reaped: bool) {
    if reaped {
        return;
    }
    stop_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn stop_group(child: &Child) {
    if let Ok(group) = i32::try_from(child.id()) {
        // SAFETY: kill only sends a signal; the group is the child's own,
        // created by process_group(0), and the child is not yet reaped, so
        // its ID cannot name another group.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
fn stop_group(child: &Child) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let taskkill = PathBuf::from(system).join("System32").join("taskkill.exe");
    let _ = Command::new(taskkill)
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

/// Turns chunks of bytes into text without splitting a character between
/// two chunks; invalid bytes become U+FFFD.
#[derive(Default)]
pub struct Utf8 {
    pending: Vec<u8>,
}

impl Utf8 {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.pending.clear();
                    return text;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    text.push_str(std::str::from_utf8(&self.pending[..valid]).unwrap_or_default());
                    match error.error_len() {
                        // An incomplete character at the end waits for the
                        // next chunk.
                        None => {
                            self.pending.drain(..valid);
                            return text;
                        }
                        Some(length) => {
                            text.push('\u{FFFD}');
                            self.pending.drain(..valid + length);
                        }
                    }
                }
            }
        }
    }

    /// What is left at the end of the output.
    pub fn finish(&mut self) -> String {
        let rest = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_keeps_a_character_split_between_chunks() {
        let mut decoder = Utf8::default();
        let bytes = "é ok".as_bytes();
        assert_eq!(decoder.push(&bytes[..1]), "");
        assert_eq!(decoder.push(&bytes[1..]), "é ok");
        assert_eq!(decoder.push(&[0xff, b'a']), "\u{FFFD}a");
        assert_eq!(decoder.push(&[0xe2, 0x82]), "");
        assert_eq!(decoder.finish(), "\u{FFFD}");
    }

    #[cfg(unix)]
    fn shell(script: &str) -> Invocation {
        Invocation {
            program: "/bin/sh".into(),
            arguments: vec!["-c".into(), script.into()],
            directory: std::env::temp_dir(),
            environment: vec![("RUST_SUPPORT_TEST".into(), "value".into())],
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_answers_the_exit_code_and_the_output() {
        let mut output = Vec::new();
        let code = run(
            &shell("printf out; printf \"$RUST_SUPPORT_TEST\" >&2; exit 3"),
            &Cancellation::new(),
            |pipe, bytes| {
                output.push((pipe, bytes.to_vec()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(code, 3);
        assert!(output.contains(&(Pipe::Stdout, b"out".to_vec())));
        assert!(output.contains(&(Pipe::Stderr, b"value".to_vec())));
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_stops_the_program_and_its_children() {
        let cancellation = Cancellation::new();
        let later = cancellation.clone();
        let started = Instant::now();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            later.cancel();
        });
        let result = run(&shell("sleep 30 & sleep 30"), &cancellation, |_, _| Ok(()));
        canceller.join().unwrap();
        assert!(matches!(result, Err(FunctionError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[test]
    fn an_error_from_the_output_stops_the_program() {
        let started = Instant::now();
        let result = run(&shell("echo x; sleep 30"), &Cancellation::new(), |_, _| {
            Err(PluginError::new("test.stop", "stop").into())
        });
        assert!(matches!(result, Err(FunctionError::Plugin(error)) if error.code() == "test.stop"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
