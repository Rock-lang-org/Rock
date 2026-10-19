use super::{BalancedGroups, Delimiter, GroupError, GroupLimits, StructuralGroup};
use crate::lexer::lossless::{LosslessStream, ScanLimits};

const STORAGE: ScanLimits = ScanLimits {
    max_source_bytes: 1024 * 1024,
    max_runs: 64 * 1024,
};
const GROUPS: GroupLimits = GroupLimits { max_depth: 128, max_groups: 1024 };

fn scan(source: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/macro-a0-07.rk".into(), source.as_bytes().to_vec(), STORAGE)
        .unwrap()
}

fn text(stream: &LosslessStream) -> String {
    stream.runs().iter().map(|run| run.text()).collect()
}

#[test]
fn macro_a0_07_requested_nested_groups_keep_exact_ranges_and_origins() {
    let source = scan("(é [42] {x})");
    let groups = BalancedGroups::parse(&source, GROUPS).unwrap();
    assert_eq!(groups.records().len(), 3);
    let outer = groups.group(0).unwrap();
    assert_eq!(outer.delimiter(), Delimiter::Paren);
    assert_eq!(text(&outer.whole(STORAGE).unwrap()), "(é [42] {x})");
    assert_eq!(text(&outer.inner(STORAGE).unwrap()), "é [42] {x}");
    assert_eq!(outer.open(STORAGE).unwrap().runs()[0].range(), 0..1);
    assert_eq!(outer.close(STORAGE).unwrap().runs()[0].range(), 12..13);
}

#[test]
fn macro_a0_07_neutral_unmatched_delimiters_only_fail_requested_rock_view() {
    let source = scan("([}");
    assert!(matches!(BalancedGroups::parse(&source, GROUPS), Err(GroupError::MismatchedClose { .. })));
    assert_eq!(text(&source), "([}");
    let wrapped = StructuralGroup::from_parts(
        Delimiter::Paren, &scan("("), &source, &scan(")"), STORAGE,
    ).unwrap();
    assert_eq!(text(wrapped.stream()), "(([})");
}

#[test]
fn macro_a0_07_group_arena_tracks_parents_without_recursive_trees() {
    let source = scan("({\"not ) ]\" [x /* } */]})");
    let groups = BalancedGroups::parse(&source, GROUPS).unwrap();
    assert_eq!(groups.records().iter().map(|group| (group.delimiter, group.parent)).collect::<Vec<_>>(), vec![
        (Delimiter::Paren, None), (Delimiter::Brace, Some(0)), (Delimiter::Bracket, Some(1)),
    ]);
    assert_eq!(text(&groups.group(2).unwrap().inner(STORAGE).unwrap()), "x /* } */");
    assert!(groups.group(3).is_none());
    for index in 0..groups.records().len() {
        let group = groups.group(index).unwrap();
        let whole = group.whole(STORAGE).unwrap();
        assert_eq!(group.open(STORAGE).unwrap().len_bytes(), 1);
        assert_eq!(group.close(STORAGE).unwrap().len_bytes(), 1);
        assert_eq!(whole.len_bytes(), group.inner(STORAGE).unwrap().len_bytes() + 2);
    }
}

#[test]
fn macro_a0_07_malformed_requests_report_the_exact_delimiter_locations() {
    let unexpected = scan("x)");
    assert!(matches!(
        BalancedGroups::parse(&unexpected, GROUPS),
        Err(GroupError::UnexpectedClose { delimiter: Delimiter::Paren, span }) if span.start == 1 && span.end == 2,
    ));
    let mismatched = scan("([)]");
    assert!(matches!(
        BalancedGroups::parse(&mismatched, GROUPS),
        Err(GroupError::MismatchedClose { expected: Delimiter::Bracket, actual: Delimiter::Paren, open, close })
            if open.start == 1 && open.end == 2 && close.start == 2 && close.end == 3,
    ));
    let unclosed = scan("([x");
    assert!(matches!(
        BalancedGroups::parse(&unclosed, GROUPS),
        Err(GroupError::Unclosed { delimiter: Delimiter::Bracket, open }) if open.start == 1 && open.end == 2,
    ));
    assert_eq!(text(&unexpected), "x)");
    assert_eq!(text(&mismatched), "([)]");
    assert_eq!(text(&unclosed), "([x");
}

#[test]
fn macro_a0_07_group_views_respect_the_supplied_slice_and_its_source_extent() {
    let original = scan("prefix (é) suffix");
    let bounded = original.slice(7..10, STORAGE).unwrap();
    assert!(matches!(
        BalancedGroups::parse(&bounded, GROUPS),
        Err(GroupError::Unclosed { delimiter: Delimiter::Paren, open }) if open.start == 7 && open.end == 8,
    ));
    let complete = original.slice(7..11, STORAGE).unwrap();
    let groups = BalancedGroups::parse(&complete, GROUPS).unwrap();
    let group = groups.group(0).unwrap();
    assert_eq!(group.open(STORAGE).unwrap().runs()[0].range(), 7..8);
    assert_eq!(group.close(STORAGE).unwrap().runs()[0].range(), 10..11);
    assert_eq!(text(&group.inner(STORAGE).unwrap()), "é");
}

#[test]
fn macro_a0_07_macro_repetition_markers_do_not_hide_parentheses() {
    let source = scan("$(x)*");
    let groups = BalancedGroups::parse(&source, GROUPS).unwrap();
    assert_eq!(groups.records().len(), 1);
    let group = groups.group(0).unwrap();
    assert_eq!(text(&group.whole(STORAGE).unwrap()), "(x)");
    assert_eq!(group.open(STORAGE).unwrap().runs()[0].range(), 1..2);
    assert_eq!(group.close(STORAGE).unwrap().runs()[0].range(), 3..4);
    assert_eq!(text(&source), "$(x)*");
}

#[test]
fn macro_a0_07_depth_and_group_count_are_bounded_independently() {
    use super::GroupLimit;

    let nested = scan("((x))");
    assert!(matches!(
        BalancedGroups::parse(&nested, GroupLimits { max_depth: 1, ..GROUPS }),
        Err(GroupError::LimitExceeded { limit: GroupLimit::Depth, span }) if span.start == 1,
    ));
    let sequential = scan("() []");
    assert!(matches!(
        BalancedGroups::parse(&sequential, GroupLimits { max_groups: 1, ..GROUPS }),
        Err(GroupError::LimitExceeded { limit: GroupLimit::Count, span }) if span.start == 3,
    ));
    assert_eq!(BalancedGroups::parse(&sequential, GroupLimits { max_depth: 1, max_groups: 2 }).unwrap().records().len(), 2);
    let no_groups = scan("x");
    assert!(BalancedGroups::parse(&no_groups, GroupLimits { max_depth: 0, max_groups: 0 }).unwrap().records().is_empty());
}

#[test]
fn macro_a0_07_deep_requested_groups_use_bounded_iterative_storage() {
    let depth = 4096;
    let source = scan(&format!("{}x{}", "(".repeat(depth), ")".repeat(depth)));
    let groups = BalancedGroups::parse(&source, GroupLimits { max_depth: depth, max_groups: depth }).unwrap();
    assert_eq!(groups.records().len(), depth);
    assert_eq!(text(&groups.group(depth - 1).unwrap().inner(STORAGE).unwrap()), "x");
    assert_eq!(groups.records()[depth - 1].parent, Some(depth - 2));
}

#[test]
fn macro_a0_07_constructed_groups_validate_wrappers_not_arbitrary_dsl_payload() {
    let source = scan("prefix \"<p>🦀([}");
    let body = source.slice(7..source.len_bytes(), STORAGE).unwrap();
    let group = StructuralGroup::from_parts(Delimiter::Brace, &scan("{"), &body, &scan("}"), STORAGE).unwrap();
    assert_eq!(group.view().delimiter(), Delimiter::Brace);
    assert_eq!(text(&group.view().inner(STORAGE).unwrap()), text(&body));
    assert_eq!(group.view().inner(STORAGE).unwrap().runs()[0].range().start, 7);
    assert!(matches!(BalancedGroups::parse(group.stream(), GROUPS), Err(GroupError::Rock(_))));
    assert_eq!(group.view().open(STORAGE).unwrap().runs()[0].text(), "{");
    assert_eq!(group.view().close(STORAGE).unwrap().runs()[0].text(), "}");
    let empty = StructuralGroup::from_parts(Delimiter::Bracket, &scan("["), &scan(""), &scan("]"), STORAGE).unwrap();
    assert_eq!(text(empty.stream()), "[]");
    assert_eq!(empty.view().inner(STORAGE).unwrap().len_bytes(), 0);
}

#[test]
fn macro_a0_07_structural_wrappers_and_storage_limits_are_checked() {
    for (open, close, opening) in [("[", ")", true), ("(", "]", false), ("", ")", true), ("((", ")", true), ("🦀", ")", true)] {
        assert!(matches!(
            StructuralGroup::from_parts(Delimiter::Paren, &scan(open), &scan("x"), &scan(close), STORAGE),
            Err(GroupError::InvalidWrapper { delimiter: Delimiter::Paren, opening: actual, .. }) if actual == opening,
        ));
    }
    assert!(matches!(
        StructuralGroup::from_parts(Delimiter::Paren, &scan("("), &scan("🦀"), &scan(")"), ScanLimits { max_source_bytes: 5, ..STORAGE }),
        Err(GroupError::Preservation(crate::lexer::lossless::LosslessError::SourceTooLarge { .. })),
    ));
    assert!(matches!(
        StructuralGroup::from_parts(Delimiter::Paren, &scan("("), &scan("x"), &scan(")"), ScanLimits { max_runs: 2, ..STORAGE }),
        Err(GroupError::Preservation(crate::lexer::lossless::LosslessError::TooManyRuns { .. })),
    ));
}

#[test]
fn macro_a0_07_group_slices_retain_different_source_origins() {
    use crate::lexer::lossless::JoinBoundary;

    let left = scan("(a");
    let right = LosslessStream::from_bytes("/virtual/right.rk".into(), b"[b])".to_vec(), STORAGE).unwrap();
    let joined = left.join(&right, JoinBoundary::Separate, STORAGE).unwrap();
    let groups = BalancedGroups::parse(&joined, GROUPS).unwrap();
    let outer = groups.group(0).unwrap();
    assert_eq!(text(&outer.whole(STORAGE).unwrap()), "(a[b])");
    assert_eq!(outer.close(STORAGE).unwrap().runs()[0].span().file_path, std::path::PathBuf::from("/virtual/right.rk"));
    let inner = outer.inner(STORAGE).unwrap();
    assert_eq!(inner.runs()[1].boundary(), JoinBoundary::Separate);
    assert_eq!(groups.group(1).unwrap().open(STORAGE).unwrap().runs()[0].range(), 0..1);
}

#[test]
fn macro_a0_07_shared_delimiters_flatten_existing_macro_wrappers_with_origins() {
    use crate::lexer::{Span, TokenType};
    use crate::macro_expansion::{TokenOrigin, TokenStream, TokenTree};

    let open = Span::new("/virtual/open.rk".into(), 7, 8);
    let close = Span::new("/virtual/close.rk".into(), 3, 4);
    let tree = TokenStream { trees: vec![TokenTree::Delimited {
        delimiter: Delimiter::Brace,
        open_origin: TokenOrigin::Source(open.clone()),
        inner: TokenStream::default(),
        close_origin: TokenOrigin::Source(close.clone()),
    }] };
    let tokens = tree.to_tokens_with_origins();
    assert_eq!(tokens[0].0.token_type, TokenType::OpenBrace);
    assert_eq!(tokens[0].0.span, open);
    assert_eq!(tokens[1].0.token_type, TokenType::CloseBrace);
    assert_eq!(tokens[1].0.span, close);
}
