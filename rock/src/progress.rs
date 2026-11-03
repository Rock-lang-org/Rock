use std::{
    fmt,
    io::{self, IsTerminal, Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc::{self, RecvTimeoutError, SyncSender},
    thread,
    time::{Duration, Instant},
};

use crate::package::{BuildProfile, Package};

pub(crate) struct BuildProgress {
    completed: usize,
    total: usize,
    started: Instant,
}

impl Default for BuildProgress {
    fn default() -> Self {
        Self::new(0)
    }
}

impl BuildProgress {
    pub(crate) fn new(total: usize) -> Self {
        Self {
            completed: 0,
            total,
            started: Instant::now(),
        }
    }

    pub(crate) fn complete(&mut self) {
        debug_assert!(self.completed < self.total || self.total == 0);
        if self.completed < self.total {
            self.completed += 1;
        }
    }

    fn percent(&self) -> usize {
        self.completed
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(0)
    }
}

pub(crate) fn compiling(package: &Package) {
    status(
        "◆",
        "compile",
        &package.manifest.crate_.name,
        format_args!(
            "v{} · {}",
            package.manifest.crate_.version,
            package.root_dir.display(),
        ),
    );
}

pub(crate) fn cleaned(removed: bool) {
    status(
        "✓",
        "clean",
        "build",
        format_args!("· {}", if removed { "removed" } else { "already clean" }),
    );
}

pub(crate) fn finished(profile: BuildProfile, elapsed: Duration, progress: &BuildProgress) {
    let description = match profile {
        BuildProfile::Debug => "unoptimized",
        BuildProfile::Release => "optimized",
    };
    status(
        "✓",
        "ready",
        profile.directory(),
        format_args!(
            "· {} · {}/{} crates · {}% · {:.2}s",
            description,
            progress.completed,
            progress.total,
            progress.percent(),
            elapsed.as_secs_f64(),
        ),
    );
}

fn status(marker: &str, label: &str, subject: &str, details: fmt::Arguments<'_>) {
    let stderr = io::stderr();
    let color = interactive() && std::env::var_os("NO_COLOR").is_none();
    // Reporting must not turn a successful build into a failure if stderr closes.
    let _ = write_status(&mut stderr.lock(), color, marker, label, subject, details);
}

fn interactive() -> bool {
    io::stderr().is_terminal() && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

/// Keep compiler output above a live last line without changing either output stream.
pub(crate) fn run_command(
    command: &mut Command,
    package: &Package,
    progress: Option<&BuildProgress>,
) -> io::Result<ExitStatus> {
    if !interactive() {
        return command.status();
    }

    // Ariadne renders source diagnostics to stdout; preserve its original TTY
    // color decision after replacing that stream with a forwarding pipe.
    if io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("CLICOLOR").is_none_or(|value| value != "0")
    {
        command.env("CLICOLOR_FORCE", "1");
    }

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
        // Both pipes are guaranteed by spawn; still reap the child if unavailable.
        return child.wait();
    };
    let (sender, receiver) = mpsc::sync_channel(32);
    let mut line = LiveLine::default();
    let started = Instant::now();
    let mut frame = 0;
    let mut streams_closed = 0;
    let mut status = None;
    let mut error = None;
    let mut at_line_start = true;
    let mut render_enabled = true;
    let color = std::env::var_os("NO_COLOR").is_none();

    thread::scope(|scope| {
        let stdout_sender = sender.clone();
        scope.spawn(move || forward_output(stdout, true, stdout_sender));
        let stderr_sender = sender.clone();
        scope.spawn(move || forward_output(stderr, false, stderr_sender));
        scope.spawn(move || {
            let _ = sender.send(Event::Exited(child.wait()));
        });

        while streams_closed < 2 || status.is_none() {
            // Only redraw between complete lines, never over a partial diagnostic.
            if at_line_start && render_enabled {
                let elapsed = progress
                    .map_or_else(|| started.elapsed(), |progress| progress.started.elapsed());
                if line.draw(package, progress, frame, elapsed, color).is_err() {
                    render_enabled = false;
                }
                frame += 1;
            }
            match receiver.recv_timeout(Duration::from_millis(90)) {
                Ok(Event::Output { stdout, bytes }) => {
                    let _ = line.clear();
                    let result = if stdout {
                        let mut output = io::stdout().lock();
                        output.write_all(&bytes).and_then(|_| output.flush())
                    } else {
                        let mut output = io::stderr().lock();
                        output.write_all(&bytes).and_then(|_| output.flush())
                    };
                    if let Err(err) = result {
                        error.get_or_insert(err);
                        render_enabled = false;
                    }
                    at_line_start = bytes.ends_with(b"\n");
                }
                Ok(Event::Closed(result)) => {
                    streams_closed += 1;
                    if let Err(err) = result {
                        error.get_or_insert(err);
                    }
                }
                Ok(Event::Exited(result)) => status = Some(result),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    let _ = line.clear();
    if let Some(error) = error {
        return Err(error);
    }
    status.unwrap_or_else(|| Err(io::Error::other("Compiler status channel closed")))
}

enum Event {
    Output { stdout: bool, bytes: Vec<u8> },
    Closed(io::Result<()>),
    Exited(io::Result<ExitStatus>),
}

fn forward_output(mut reader: impl Read, stdout: bool, sender: SyncSender<Event>) {
    let mut buffer = [0; 4096];
    let result = loop {
        match reader.read(&mut buffer) {
            Ok(0) => break Ok(()),
            Ok(length) => {
                if sender
                    .send(Event::Output {
                        stdout,
                        bytes: buffer[..length].to_vec(),
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => break Err(error),
        }
    };
    let _ = sender.send(Event::Closed(result));
}

#[derive(Default)]
struct LiveLine {
    visible: bool,
}

impl LiveLine {
    fn draw(
        &mut self,
        package: &Package,
        progress: Option<&BuildProgress>,
        frame: usize,
        elapsed: Duration,
        color: bool,
    ) -> io::Result<()> {
        self.visible = true;
        let mut stderr = io::stderr().lock();
        write_activity(
            &mut stderr,
            color,
            &package.manifest.crate_.name,
            package.profile,
            progress,
            frame,
            elapsed,
        )?;
        stderr.flush()
    }

    fn clear(&mut self) -> io::Result<()> {
        if self.visible {
            let mut stderr = io::stderr().lock();
            stderr.write_all(b"\r\x1b[2K")?;
            stderr.flush()?;
            self.visible = false;
        }
        Ok(())
    }
}

impl Drop for LiveLine {
    fn drop(&mut self) {
        let _ = self.clear();
    }
}

fn write_activity(
    writer: &mut impl Write,
    color: bool,
    name: &str,
    profile: BuildProfile,
    progress: Option<&BuildProgress>,
    frame: usize,
    elapsed: Duration,
) -> io::Result<()> {
    let progress = progress.filter(|progress| progress.total != 0);
    let pulse = ['◆', '◇', '◈', '◇'][frame % 4];
    let (rail, counts) = if let Some(progress) = progress {
        // Crates, not elapsed time, determine the filled portion of the bar.
        let filled = progress.completed.saturating_mul(10) / progress.total;
        let rail = (0..10)
            .map(|index| {
                if index < filled {
                    '━'
                } else if index == filled {
                    '◆'
                } else {
                    '─'
                }
            })
            .collect::<String>();
        (
            rail,
            format!(
                "{}/{} · {}% · ",
                progress.completed,
                progress.total,
                progress.percent()
            ),
        )
    } else {
        // Toolchain preparation is not part of the project crate graph.
        let position = frame % 18;
        let position = if position > 9 {
            18 - position
        } else {
            position
        };
        let rail = (0..10)
            .map(|index| if index == position { '◆' } else { '─' })
            .collect::<String>();
        (rail, String::new())
    };
    let name = name
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    // Disable wrapping for this write only, so narrow terminals keep one live row.
    write!(writer, "\r\x1b[2K\x1b[?7l")?;
    let result = if color {
        write!(
            writer,
            "  \x1b[38;5;80m{pulse} [{rail}] {counts}\x1b[0m\x1b[1;38;5;147m{name}\x1b[0m \x1b[2;38;5;245m· {} · {:.1}s\x1b[0m",
            profile.directory(), elapsed.as_secs_f64(),
        )
    } else {
        write!(
            writer,
            "  {pulse} [{rail}] {counts}{name} · {} · {:.1}s",
            profile.directory(),
            elapsed.as_secs_f64()
        )
    };
    let restore = writer.write_all(b"\x1b[?7h");
    result.and(restore)
}

fn write_status(
    writer: &mut impl Write,
    color: bool,
    marker: &str,
    label: &str,
    subject: &str,
    details: fmt::Arguments<'_>,
) -> io::Result<()> {
    if color {
        writeln!(
            writer,
            "  \x1b[1;38;5;80m{marker} {label:<7}\x1b[0m \x1b[1;38;5;147m{subject}\x1b[0m \x1b[2;38;5;245m{details}\x1b[0m",
        )
    } else {
        writeln!(writer, "  {marker} {label:<7} {subject} {details}")
    }
}

#[cfg(test)]
mod tests;
