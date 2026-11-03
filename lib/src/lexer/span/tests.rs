use super::Span;

#[test]
fn span_equality_compares_path_and_range() {
    assert_eq!(
        Span::new("/virtual/main.rk".into(), 1, 3),
        Span::new("/virtual/main.rk".into(), 1, 3)
    );
    assert_ne!(
        Span::new("/virtual/main.rk".into(), 1, 3),
        Span::new("/virtual/main.rk".into(), 2, 3)
    );
}
