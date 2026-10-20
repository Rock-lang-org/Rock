//! Bounded, single-threaded pipe pumping. No IO thread can outlive its worker
//! or leave the compiler blocked in a join after the process has exited.

use std::io::{ErrorKind, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{decode_response, ProcMacroArtifact, ProcMacroRequest, ProcMacroResponse, MAX_PROC_MACRO_MESSAGE_BYTES};

#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    // The descriptors remain owned by the child pipe handles for this call.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(format!("failed to configure worker pipe: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(unix)]
struct Worker {
    child: Child,
    group: Option<libc::pid_t>,
}

#[cfg(unix)]
impl Worker {
    fn stop_group(&mut self) {
        if let Some(group) = self.group.take() { unsafe { libc::kill(-group, libc::SIGKILL); } }
    }

    fn status(&mut self) -> Result<Option<std::process::ExitStatus>, String> {
        // Observe exit without reaping: the group ID must stay reserved until
        // descendants are killed, not be reused by an unrelated host process.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe { libc::waitid(libc::P_PID, self.child.id() as libc::id_t, &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == ErrorKind::Interrupted { Ok(None) } else { Err(format!("failed observing worker exit: {error}")) };
        }
        if info.si_signo == 0 { return Ok(None); }
        self.stop_group();
        self.child.try_wait().map_err(|error| format!("failed waiting for worker: {error}"))
    }
}

#[cfg(unix)]
impl Drop for Worker {
    fn drop(&mut self) {
        // Each worker has its own process group. Always close its descendants,
        // including successful workers that leave inherited pipe handles open.
        self.stop_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_available(reader: &mut impl Read, bytes: &mut Vec<u8>, name: &str) -> Result<bool, String> {
    let mut buffer = [0; 8192];
    // Fairness matters when a worker floods stdout and stderr simultaneously.
    for _ in 0..8 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(length) => {
                if bytes.len() + length > MAX_PROC_MACRO_MESSAGE_BYTES as usize {
                    return Err(format!("{name} exceeded maximum proc macro message size of {MAX_PROC_MACRO_MESSAGE_BYTES} bytes"));
                }
                bytes.extend_from_slice(&buffer[..length]);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("failed reading worker {name}: {error}")),
        }
    }
    Ok(false)
}

fn stderr_context(error: String, stderr: &[u8]) -> String {
    if stderr.is_empty() { return error; }
    let context = String::from_utf8_lossy(&stderr[..stderr.len().min(4096)]);
    format!("{error}\nworker stderr: {context}{}", if stderr.len() > 4096 { "\n[stderr truncated]" } else { "" })
}

#[cfg(unix)]
pub(super) fn run(artifact: &ProcMacroArtifact, request: &ProcMacroRequest, bytes: &[u8], started: Instant, timeout: Duration, no_proc_execution: bool, cancellation: Option<&AtomicBool>) -> Result<ProcMacroResponse, String> {
    use std::os::unix::process::CommandExt;

    if no_proc_execution { return Err("procedural macro execution is disabled (--no-proc-execution)".into()); }
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) { return Err("proc macro cancelled before spawn".into()); }
    if started.elapsed() >= timeout { return Err("proc macro timed out before spawn".into()); }
    let child = Command::new(&artifact.executable).process_group(0)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| format!("failed to start proc macro: {error}"))?;
    let group = child.id() as libc::pid_t;
    let mut child = Worker { child, group: Some(group) };
    let mut stdin = Some(child.child.stdin.take().ok_or("proc macro stdin unavailable")?);
    let mut stdout = child.child.stdout.take().ok_or("proc macro stdout unavailable")?;
    let mut stderr = child.child.stderr.take().ok_or("proc macro stderr unavailable")?;
    nonblocking(stdin.as_ref().ok_or("proc macro stdin unavailable")?)?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;

    let mut written = 0;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result: Result<ProcMacroResponse, String> = (|| {
        let mut out_closed = false;
        let mut err_closed = false;
        let mut status = None;
        loop {
            if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) { return Err("proc macro cancelled during request/response IO or shutdown".into()); }
            if started.elapsed() >= timeout { return Err("proc macro timed out during request/response IO or shutdown".into()); }
            if let Some(pipe) = &mut stdin {
                let end = bytes.len().min(written + 8192);
                match pipe.write(&bytes[written..end]) {
                    Ok(0) if written < bytes.len() => return Err("worker stdin closed during request".into()),
                    Ok(length) => written += length,
                    Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                    Err(error) => return Err(format!("failed writing worker stdin: {error}")),
                }
                if written == bytes.len() { stdin = None; }
            }
            if !out_closed { out_closed = read_available(&mut stdout, &mut out, "stdout")?; }
            if !err_closed { err_closed = read_available(&mut stderr, &mut err, "stderr")?; }
            if status.is_none() { status = child.status()?; }
            if let Some(status) = status {
                if !status.success() {
                    // The exit observation can race the first stderr read.
                    // These nonblocking drains never wait on inherited pipes.
                    if !err_closed { let _ = read_available(&mut stderr, &mut err, "stderr"); }
                    return Err(format!("proc macro exited with {status}"));
                }
                if out_closed && err_closed && stdin.is_none() {
                    let response = decode_response(&out).map_err(|error| format!("malformed proc macro response: {error}"))?;
                    response.validate_for(request)?;
                    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) { return Err("proc macro cancelled validating response".into()); }
                    if started.elapsed() >= timeout { return Err("proc macro timed out validating response".into()); }
                    return Ok(response);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    })();
    result.map_err(|error| stderr_context(error, &err))
}

#[cfg(not(unix))]
pub(super) fn run(_: &ProcMacroArtifact, _: &ProcMacroRequest, _: &[u8], _: Instant, _: Duration, _: bool, _: Option<&AtomicBool>) -> Result<ProcMacroResponse, String> {
    Err("bounded proc macro execution is unsupported on this host".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_b_02_stderr_context_is_bounded_and_handles_non_utf8() {
        let context = stderr_context("failure".into(), &vec![255; 8192]);
        assert!(context.contains("failure"));
        assert!(context.contains("truncated"));
        assert!(context.len() < 13_000);
    }
}
