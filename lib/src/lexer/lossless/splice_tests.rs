use super::{JoinBoundary, LosslessError, LosslessStream, ScanLimits, SyntaxContext};

const LIMITS: ScanLimits = ScanLimits {
    max_source_bytes: 1024 * 1024,
    max_runs: 64 * 1024,
};

fn scan(source: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/macro-a0-03.rk".into(), source.as_bytes().to_vec(), LIMITS)
        .unwrap()
}

fn text(stream: &LosslessStream) -> String {
    stream.runs().iter().map(|run| run.text()).collect()
}

#[test]
fn macro_a0_03_slices_inside_and_between_runs_preserve_origin_context() {
    let source = scan("abé \t🦀cd");
    let sliced = source.slice(1..10, LIMITS).unwrap();
    assert_eq!(text(&sliced), "bé \t🦀");
    assert_eq!(sliced.len_bytes(), 9);
    assert_eq!(sliced.runs().len(), 3);
    for (run, range) in sliced.runs().iter().zip([1..4, 4..6, 6..10]) {
        assert_eq!(run.range(), range);
        assert_eq!(run.context(), SyntaxContext::ROOT);
        assert_eq!(run.span().start, range.start);
        assert_eq!(run.span().end, range.end);
    }
    let middle = sliced.slice(1..3, LIMITS).unwrap();
    assert_eq!(text(&middle), "é");
    assert_eq!(middle.runs()[0].range(), 2..4);
}

#[test]
fn macro_a0_03_cuts_reject_out_of_range_and_non_utf8_boundaries() {
    let source = scan("a🦀b");
    assert!(matches!(source.slice(0..7, LIMITS), Err(LosslessError::InvalidRange { .. })));
    assert!(matches!(source.slice(2..5, LIMITS), Err(LosslessError::NotCharBoundary { offset: 2 })));
    let left = source.slice(0..1, LIMITS).unwrap();
    let right = source.slice(5..6, LIMITS).unwrap();
    let joined = left.join(&right, JoinBoundary::Separate, LIMITS).unwrap();
    assert_eq!(text(&joined), "ab");
    assert_eq!(joined.runs()[1].boundary(), JoinBoundary::Separate);
}

#[test]
fn macro_a0_03_reversed_overflowing_and_empty_mid_scalar_cuts_reject() {
    let source = scan("a🦀b");
    for (start, end) in [(4, 1), (0, usize::MAX), (usize::MAX, usize::MAX), (7, 7)] {
        assert!(matches!(
            source.slice(start..end, LIMITS),
            Err(LosslessError::InvalidRange { start: actual_start, end: actual_end, len: 6 })
                if start == actual_start && end == actual_end,
        ));
    }
    for range in [0..2, 0..3, 2..2, 3..5, 4..6] {
        assert!(matches!(source.slice(range, LIMITS), Err(LosslessError::NotCharBoundary { .. })));
    }
}

#[test]
fn macro_a0_03_empty_slices_keep_precise_source_points() {
    let source = scan("a🦀 b");
    for offset in [0, 1, 5, 6, 7] {
        let empty = source.slice(offset..offset, LIMITS).unwrap();
        assert_eq!(empty.len_bytes(), 0);
        assert!(empty.runs().is_empty());
        assert_eq!(empty.start_span().start, offset);
        assert_eq!(empty.start_span().end, offset);
        assert_eq!(empty.end_span(), empty.start_span());
        assert_eq!(empty.anchor.context, SyntaxContext::ROOT);
        assert!(matches!(empty.binding_context(0..0), Err(LosslessError::EmptyBindingRange)));
    }
    let empty = scan("");
    assert_eq!(empty.slice(0..0, LIMITS).unwrap().start_span(), empty.start_span());
}

#[test]
fn macro_a0_03_cross_source_join_retains_each_snapshot_and_original_spans() {
    let left = scan("prefix abc").slice(7..10, LIMITS).unwrap();
    let right = LosslessStream::from_bytes(
        "/virtual/other.rk".into(), "éxyz".as_bytes().to_vec(), LIMITS,
    ).unwrap().slice(2..5, LIMITS).unwrap();
    let joined = left.join(&right, JoinBoundary::Separate, LIMITS).unwrap();
    assert_eq!(text(&joined), "abcxyz");
    assert_eq!(joined.runs()[0].range(), 7..10);
    assert_eq!(joined.runs()[1].range(), 2..5);
    assert_eq!(joined.runs()[1].span().file_path, std::path::PathBuf::from("/virtual/other.rk"));
    assert_eq!(joined.start_span(), left.start_span());
    assert_eq!(joined.end_span(), right.end_span());
    assert!(std::sync::Arc::ptr_eq(&joined.runs()[0].source, &left.runs()[0].source));
    assert!(std::sync::Arc::ptr_eq(&joined.runs()[1].source, &right.runs()[0].source));
    let middle = joined.slice(2..5, LIMITS).unwrap();
    assert_eq!(text(&middle), "cxy");
    assert_eq!(middle.runs()[0].range(), 9..10);
    assert_eq!(middle.runs()[1].range(), 2..4);
    assert_eq!(middle.runs()[1].boundary(), JoinBoundary::Separate);
    assert_eq!(joined.slice(3..3, LIMITS).unwrap().start_span(), right.start_span());
}

#[test]
fn macro_a0_03_duplicate_capture_keeps_occurrences_and_shared_storage() {
    let capture = scan("name");
    let duplicate = capture.join(&capture, JoinBoundary::Separate, LIMITS).unwrap();
    assert_eq!(text(&duplicate), "namename");
    assert_eq!(duplicate.runs().len(), 2);
    assert_eq!(duplicate.runs()[0].span(), duplicate.runs()[1].span());
    assert_eq!(duplicate.runs()[0].context(), duplicate.runs()[1].context());
    assert!(std::sync::Arc::ptr_eq(&duplicate.runs()[0].source, &duplicate.runs()[1].source));
    assert_eq!(duplicate.runs()[1].boundary(), JoinBoundary::Separate);
    drop(capture);
    assert_eq!(text(&duplicate.slice(2..6, LIMITS).unwrap()), "mena");
}

#[test]
fn macro_a0_03_source_adjacency_requires_identity_order_and_no_prior_splice() {
    let source = scan("foobar");
    let left = source.slice(0..3, LIMITS).unwrap();
    let right = source.slice(3..6, LIMITS).unwrap();
    let adjacent = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
    assert_eq!(text(&adjacent), "foobar");
    assert_eq!(adjacent.runs().len(), 2); // Do not erase segment metadata by coalescing.
    assert_eq!(adjacent.runs()[1].boundary(), JoinBoundary::SourceAdjacent);
    assert_eq!(adjacent.binding_context(0..6).unwrap(), SyntaxContext::ROOT);

    let separated = left.join(&right, JoinBoundary::Separate, LIMITS).unwrap();
    assert_eq!(text(&separated), text(&adjacent));
    assert_eq!(separated.runs()[1].boundary(), JoinBoundary::Separate);
    let protected_right = separated.slice(3..6, LIMITS).unwrap();
    for (a, b) in [
        (&right, &left), // Reordered.
        (&left, &left), // Duplicated.
        (&left, &protected_right), // Existing splice boundary.
    ] {
        assert!(matches!(a.join(b, JoinBoundary::SourceAdjacent, LIMITS), Err(LosslessError::NotSourceAdjacent)));
    }
    let independently_loaded = scan("foobar").slice(3..6, LIMITS).unwrap();
    assert!(matches!(
        left.join(&independently_loaded, JoinBoundary::SourceAdjacent, LIMITS),
        Err(LosslessError::NotSourceAdjacent),
    ));
}

#[test]
fn macro_a0_03_removed_gaps_are_not_guessed_or_print_relexed() {
    let source = scan("foo /* ignored by a future view */ bar");
    let left = source.slice(0..3, LIMITS).unwrap();
    let end = source.len_bytes();
    let right = source.slice(end - 3..end, LIMITS).unwrap();
    assert!(matches!(left.join(&right, JoinBoundary::SourceAdjacent, LIMITS), Err(LosslessError::NotSourceAdjacent)));
    let separate = left.join(&right, JoinBoundary::Separate, LIMITS).unwrap();
    assert_eq!(text(&separate), "foobar");
    assert_eq!(separate.runs()[1].boundary(), JoinBoundary::Separate);
    assert_eq!(separate.runs()[1].range(), end - 3..end);
}

#[test]
fn macro_a0_03_context_segments_survive_splices_and_merges_must_agree() {
    let source = scan("foobar");
    let left = source.slice(0..3, LIMITS).unwrap();
    let mut right = source.slice(3..6, LIMITS).unwrap();
    // Synthetic future hygiene context, never derived from a path/span or exposed
    // as a production minting API by this preservation-only slice.
    right.runs[0].context = SyntaxContext(7);
    right.anchor.context = SyntaxContext(7);
    let mixed = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
    assert_eq!(mixed.binding_context(0..3).unwrap(), SyntaxContext::ROOT);
    assert_eq!(mixed.binding_context(3..6).unwrap(), SyntaxContext(7));
    assert!(matches!(
        mixed.binding_context(0..6),
        Err(LosslessError::ConflictingContexts { offset: 3, left: SyntaxContext::ROOT, right: SyntaxContext(7) }),
    ));
    let cross = mixed.slice(2..5, LIMITS).unwrap();
    assert_eq!(text(&cross), "oba");
    assert_eq!(cross.runs()[0].context(), SyntaxContext::ROOT);
    assert_eq!(cross.runs()[1].context(), SyntaxContext(7));
    assert!(matches!(cross.binding_context(0..3), Err(LosslessError::ConflictingContexts { offset: 1, .. })));
    assert_eq!(mixed.slice(4..4, LIMITS).unwrap().anchor.context, SyntaxContext(7));
    assert_eq!(source.runs()[0].context(), SyntaxContext::ROOT);
    assert!(matches!(source.binding_context(0..7), Err(LosslessError::InvalidRange { .. })));
}

#[test]
fn macro_a0_03_equal_source_locations_do_not_authorize_context_merging() {
    let root = scan("x");
    let mut marked = root.clone();
    marked.runs[0].context = SyntaxContext(9);
    let mixed = root.join(&marked, JoinBoundary::Separate, LIMITS).unwrap();
    assert_eq!(mixed.runs()[0].span(), mixed.runs()[1].span());
    assert!(matches!(mixed.binding_context(0..2), Err(LosslessError::ConflictingContexts { .. })));
}

#[test]
fn macro_a0_03_structural_operations_enforce_spelling_and_metadata_budgets() {
    let source = scan("a b");
    assert!(matches!(
        source.slice(0..3, ScanLimits { max_source_bytes: 2, ..LIMITS }),
        Err(LosslessError::SourceTooLarge { bytes: 3, limit: 2 }),
    ));
    assert!(matches!(
        source.slice(0..3, ScanLimits { max_runs: 2, ..LIMITS }),
        Err(LosslessError::TooManyRuns { .. }),
    ));
    assert!(matches!(
        source.join(&source, JoinBoundary::Separate, ScanLimits { max_source_bytes: 5, ..LIMITS }),
        Err(LosslessError::SourceTooLarge { bytes: 6, limit: 5 }),
    ));
    assert!(matches!(
        source.join(&source, JoinBoundary::Separate, ScanLimits { max_runs: 5, ..LIMITS }),
        Err(LosslessError::TooManyRuns { .. }),
    ));
    let exact = ScanLimits { max_source_bytes: 6, max_runs: 6 };
    assert_eq!(source.join(&source, JoinBoundary::Separate, exact).unwrap().len_bytes(), 6);
    assert!(matches!(
        source.join(&scan(""), JoinBoundary::Separate, ScanLimits { max_runs: 0, ..LIMITS }),
        Err(LosslessError::TooManyRuns { .. }),
    ));
}

#[test]
fn macro_a0_03_empty_splices_do_not_add_or_erase_boundaries() {
    let source = scan("a b");
    let empty = source.slice(1..1, LIMITS).unwrap();
    for joined in [
        source.join(&empty, JoinBoundary::Separate, LIMITS).unwrap(),
        empty.join(&source, JoinBoundary::Separate, LIMITS).unwrap(),
    ] {
        assert_eq!(text(&joined), "a b");
        assert_eq!(joined.runs().len(), source.runs().len());
        assert_eq!(joined.start_span(), source.start_span());
        assert_eq!(joined.end_span(), source.end_span());
        assert_eq!(joined.runs()[0].boundary(), source.runs()[0].boundary());
    }
    let both_empty = empty.join(&scan(""), JoinBoundary::Separate, LIMITS).unwrap();
    assert_eq!(both_empty.start_span(), empty.start_span());
    assert!(both_empty.runs().is_empty());
}

#[test]
fn macro_a0_03_every_utf8_cut_round_trips_without_losing_origin_segments() {
    let spelling = "abé\t🦀\r\n`/*([\0 xyz";
    let source = scan(spelling);
    for cut in (0..=spelling.len()).filter(|offset| spelling.is_char_boundary(*offset)) {
        let left = source.slice(0..cut, LIMITS).unwrap();
        let right = source.slice(cut..spelling.len(), LIMITS).unwrap();
        let joined = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
        assert_eq!(text(&joined), spelling);
        let mut offset = 0;
        for run in joined.runs() {
            assert_eq!(run.range().start, offset);
            assert_eq!(run.context(), SyntaxContext::ROOT);
            offset = run.range().end;
        }
        assert_eq!(offset, spelling.len());
        assert_eq!(joined.start_span(), source.start_span());
        assert_eq!(joined.end_span(), source.end_span());
    }
}

#[test]
fn macro_a0_04_identifier_interpretation_cannot_remint_a_conflicting_context() {
    use crate::lexer::interpretation::{RockError, RockView};

    let source = scan("foobar");
    let left = source.slice(0..3, LIMITS).unwrap();
    let mut right = source.slice(3..6, LIMITS).unwrap();
    right.runs[0].context = SyntaxContext(8);
    let mixed = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
    let mut view = RockView::new(&mixed);
    assert!(matches!(
        view.next_piece(),
        Err(RockError::Preservation(LosslessError::ConflictingContexts {
            offset: 3, left: SyntaxContext::ROOT, right: SyntaxContext(8),
        })),
    ));
    assert_eq!(view.offset(), 0);
    let separated = left.join(&right, JoinBoundary::Separate, LIMITS).unwrap();
    let mut view = RockView::new(&separated);
    assert_eq!(view.next_piece().unwrap().unwrap().context, Some(SyntaxContext::ROOT));
    assert_eq!(view.next_piece().unwrap().unwrap().context, Some(SyntaxContext(8)));
}
