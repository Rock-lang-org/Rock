use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const MAX_BATCH: usize = 64 * 1024 * 1024;
const MAX_CASE: usize = 1048577;

pub(super) fn run_client(path: &Path) -> Output { run_client_with_input(path, &[]) }

pub(super) fn run_client_with_input(path: &Path, input: &[u8]) -> Output {
    assert!(input.len() <= MAX_CASE, "individual protocol/IO controls remain bounded");
    run(path, input, MAX_CASE, if input.len() <= 33 { 2 } else { 257 })
}

fn run(path: &Path, input: &[u8], limit: usize, chunk_size: usize) -> Output {
    let mut child = Command::new(path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let stdout = child.stdout.take().unwrap();
    let stdout = std::thread::spawn(move || {
        let mut bytes = Vec::new(); stdout.take(limit as u64 + 1).read_to_end(&mut bytes).unwrap(); bytes
    });
    let stderr = child.stderr.take().unwrap();
    let stderr = std::thread::spawn(move || {
        let mut bytes = Vec::new(); stderr.take(65537).read_to_end(&mut bytes).unwrap(); bytes
    });
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        for chunk in input.chunks(chunk_size) {
            if let Err(error) = stdin.write_all(chunk) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}"); break;
            }
        }
    });
    loop {
        if child.try_wait().unwrap().is_some() {
            writer.join().unwrap();
            let mut output = child.wait_with_output().unwrap();
            output.stdout = stdout.join().unwrap(); output.stderr = stderr.join().unwrap();
            assert!(output.stdout.len() <= limit && output.stderr.len() <= 65536, "SDK client output limit exceeded");
            return output;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let mut output = child.wait_with_output().unwrap();
            writer.join().unwrap();
            output.stdout = stdout.join().unwrap(); output.stderr = stderr.join().unwrap();
            panic!("SDK client exceeded 30s: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn encode_batch(cases: &[Vec<u8>]) -> Vec<u8> {
    assert!(cases.len() <= 1024);
    let mut bytes = Vec::new();
    for case in cases {
        assert!(case.len() <= MAX_CASE);
        assert!(bytes.len() + 4 + case.len() <= MAX_BATCH);
        bytes.extend_from_slice(&(case.len() as u32).to_le_bytes()); bytes.extend_from_slice(case);
    }
    bytes
}

fn decode_batch(mut bytes: &[u8], count: usize) -> Result<Vec<(u8, Vec<u8>)>, &'static str> {
    let mut frames = Vec::new();
    for _ in 0..count {
        if bytes.len() < 5 { return Err("truncated batch response header"); }
        let status = bytes[0];
        let length = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
        bytes = &bytes[5..];
        if !matches!(status, 0 | 2) || length > MAX_CASE || length > bytes.len() || (status == 2 && length != 0) {
            return Err("invalid batch response frame");
        }
        frames.push((status, bytes[..length].to_vec())); bytes = &bytes[length..];
    }
    if !bytes.is_empty() { return Err("trailing batch response data"); }
    Ok(frames)
}

pub(super) fn run_batch(path: &Path, cases: &[Vec<u8>]) -> Vec<(u8, Vec<u8>)> {
    let output = run(path, &encode_batch(cases), MAX_BATCH, 65536);
    assert!(output.status.success() && output.stderr.is_empty(), "batch client failed: {output:?}");
    decode_batch(&output.stdout, cases.len()).unwrap()
}

#[test]
fn macro_batch_response_framing_rejects_partial_invalid_and_trailing_data() {
    assert_eq!(decode_batch(&[0, 1, 0, 0, 0, 42, 2, 0, 0, 0, 0], 2).unwrap(), vec![(0, vec![42]), (2, vec![])]);
    for bytes in [&[0, 1, 0, 0][..], &[0, 1, 0, 0, 0], &[1, 0, 0, 0, 0], &[2, 1, 0, 0, 0, 42], &[0, 0, 0, 0, 0, 42]] {
        assert!(decode_batch(bytes, 1).is_err(), "{bytes:?}");
    }
}

#[test]
fn macro_batch_request_framing_preserves_empty_and_binary_cases() {
    assert_eq!(encode_batch(&[vec![], vec![0, 255]]), [0, 0, 0, 0, 2, 0, 0, 0, 0, 255]);
}
