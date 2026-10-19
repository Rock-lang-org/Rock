use super::{JoinBoundary, LosslessStream, ScanLimits};
use bincode::Options;

const LIMITS: ScanLimits = ScanLimits { max_source_bytes: 65536, max_runs: 4096 };

fn source(text: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/macro-a-01.rk".into(), text.as_bytes().to_vec(), LIMITS).unwrap()
}

#[test]
fn macro_a_01_wire_roundtrip_preserves_exact_spelling_and_snapshot_adjacency() {
    let input = source("prefix 🦀\r\n \"broken /* [ suffix");
    let left = input.slice(7..11, LIMITS).unwrap();
    let right = input.slice(11..input.len_bytes(), LIMITS).unwrap();
    let joined = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
    let bytes = bincode::DefaultOptions::new().with_limit(65536).serialize(&joined).unwrap();
    let decoded: LosslessStream = bincode::DefaultOptions::new().with_limit(65536).deserialize(&bytes).unwrap();
    assert_eq!(decoded.runs().iter().map(|run| run.text()).collect::<String>(), "🦀\r\n \"broken /* [ suffix");
    assert_eq!(decoded.start_span(), joined.start_span());
    assert_eq!(decoded.end_span(), joined.end_span());
    let first = decoded.slice(0..4, LIMITS).unwrap();
    let rest = decoded.slice(4..decoded.len_bytes(), LIMITS).unwrap();
    assert!(first.join(&rest, JoinBoundary::SourceAdjacent, LIMITS).is_ok());
}

#[test]
fn macro_a_01_wire_never_infers_adjacency_from_equal_source_text() {
    let left = source("ab").slice(0..1, LIMITS).unwrap();
    let right = source("ab").slice(1..2, LIMITS).unwrap();
    let input = left.join(&right, JoinBoundary::Separate, LIMITS).unwrap();
    let bytes = bincode::serialize(&input).unwrap();
    let decoded: LosslessStream = bincode::deserialize(&bytes).unwrap();
    let left = decoded.slice(0..1, LIMITS).unwrap();
    let right = decoded.slice(1..2, LIMITS).unwrap();
    assert!(left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).is_err());
    assert_eq!(decoded.runs()[1].boundary(), JoinBoundary::Separate);
}
