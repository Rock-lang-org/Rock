use std::{
    io::{self, Read},
    process::Command,
    sync::mpsc,
    time::Duration,
};

use crate::package::BuildProfile;
use crate::tests::support::{load_package, temp_test_dir, write_package};

use super::{forward_output, run_command, write_activity, write_status, BuildProgress, Event};

#[test]
fn test_progress_command_preserves_child_exit_status() {
    let root = temp_test_dir("progress_child_status");
    write_package(&root, "app", "src/main.rk", &[], "main = !-> ()\n");
    let package = load_package(root.clone());
    let status = run_command(Command::new("sh").args(["-c", "exit 7"]), &package, None).unwrap();
    assert_eq!(status.code(), Some(7));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn test_toolchain_preparation_remains_indeterminate() {
    for (frame, pulse, rail) in [
        (0, '◆', "◆─────────"),
        (3, '◇', "───◆──────"),
        (12, '◆', "──────◆───"),
    ] {
        let mut output = Vec::new();
        write_activity(
            &mut output,
            false,
            "app",
            BuildProfile::Release,
            None,
            frame,
            Duration::from_millis(1250),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("\r\x1b[2K\x1b[?7l  {pulse} [{rail}] app · release · 1.2s\x1b[?7h"),
        );
    }
}

#[test]
fn test_overall_progress_advances_only_for_completed_crates() {
    let mut progress = BuildProgress::new(4);
    for (completed, rail) in [
        (0, "◆─────────"),
        (1, "━━◆───────"),
        (2, "━━━━━◆────"),
        (3, "━━━━━━━◆──"),
        (4, "━━━━━━━━━━"),
    ] {
        for frame in [0, 5] {
            let mut output = Vec::new();
            write_activity(
                &mut output,
                false,
                "app",
                BuildProfile::Debug,
                Some(&progress),
                frame,
                Duration::from_secs(2),
            )
            .unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(
                output.contains(&format!(
                    "[{rail}] {completed}/4 · {}% · app",
                    completed * 25
                )),
                "{output}"
            );
        }
        if completed < 4 {
            progress.complete();
        }
    }
}

#[test]
fn test_activity_sanitizes_terminal_controls_in_crate_names() {
    let mut output = Vec::new();
    write_activity(
        &mut output,
        true,
        "app\r\n\x1b",
        BuildProfile::Debug,
        None,
        0,
        Duration::ZERO,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("\x1b[1;38;5;147mapp\x1b[0m"));
    assert!(!output.contains('\n'));
    assert!(output.ends_with("\x1b[?7h"));
}

#[test]
fn test_output_forwarding_preserves_bytes_and_partial_lines() {
    for stdout in [true, false] {
        let bytes = vec![b'x'; 9000];
        let (sender, receiver) = mpsc::sync_channel(32);
        forward_output(io::Cursor::new(&bytes), stdout, sender);
        let mut output = Vec::new();
        let mut closed = false;
        for event in receiver {
            match event {
                Event::Output {
                    stdout: stream,
                    bytes,
                } => {
                    assert_eq!(stream, stdout);
                    output.extend(bytes);
                }
                Event::Closed(result) => {
                    result.unwrap();
                    closed = true;
                }
                Event::Exited(_) => panic!("reader must not send a process status"),
            }
        }
        assert_eq!(output, bytes);
        assert!(closed);
    }
}

#[test]
fn test_output_forwarding_retries_interrupted_reads_and_reports_errors() {
    struct FailingReader(bool);
    impl Read for FailingReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            let kind = if self.0 {
                io::ErrorKind::Other
            } else {
                io::ErrorKind::Interrupted
            };
            self.0 = true;
            Err(io::Error::from(kind))
        }
    }
    let (sender, receiver) = mpsc::sync_channel(32);
    forward_output(FailingReader(false), false, sender);
    match receiver.recv().unwrap() {
        Event::Closed(Err(error)) => assert_eq!(error.kind(), io::ErrorKind::Other),
        _ => panic!("expected the reader error"),
    }
}

#[test]
fn test_progress_is_plain_text_when_redirected() {
    let mut output = Vec::new();
    write_status(
        &mut output,
        false,
        "◆",
        "compile",
        "dep",
        format_args!("v0.1.0 · /dep"),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "  ◆ compile dep v0.1.0 · /dep\n"
    );
}

#[test]
fn test_progress_uses_teal_lavender_and_muted_metadata() {
    let mut output = Vec::new();
    write_status(
        &mut output,
        true,
        "✓",
        "ready",
        "debug",
        format_args!("· 0.42s"),
    )
    .unwrap();
    assert_eq!(
            String::from_utf8(output).unwrap(),
            "  \x1b[1;38;5;80m✓ ready  \x1b[0m \x1b[1;38;5;147mdebug\x1b[0m \x1b[2;38;5;245m· 0.42s\x1b[0m\n",
        );
}
