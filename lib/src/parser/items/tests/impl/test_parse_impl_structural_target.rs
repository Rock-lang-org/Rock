use crate::ast::ParseType;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn borrowed_slice_inherent_impl_headers() {
    for (header, mutable) in [("&[T]", false), ("&mut [T]", true)] {
        let input = format!("impl {header}\n    @len: I64\n    @len = -> ~ArrayLen *self\n");
        let tokens = lex_test(&input);
        let config = Config::default();
        let (rest, implementation) = r#impl.process(ParseCtx::from(&tokens, &config)).unwrap();
        assert!(rest.is_empty());
        assert!(implementation.for_.is_none());
        match implementation.name {
            ParseType::Reference { is_mut, pointee } => {
                assert_eq!(is_mut, mutable);
                assert!(matches!(*pointee, ParseType::Slice(_)));
            }
            other => panic!("expected borrowed slice target, got {other:?}"),
        }
        assert_eq!(implementation.methods.len(), 1);
        assert_eq!(implementation.signatures.len(), 1);
    }
}

#[test]
fn structural_inherent_impl_headers() {
    for header in ["&Str", "[T; 3]", "*T", "(T, U)"] {
        let input = format!("impl {header}\n");
        let tokens = lex_test(&input);
        let config = Config::default();
        let (rest, implementation) = r#impl.process(ParseCtx::from(&tokens, &config)).unwrap();
        assert!(rest.is_empty());
        assert_eq!(implementation.name.type_name(), header);
    }
}

#[test]
fn structural_trait_header_is_rejected() {
    let tokens = lex_test("impl &[T] for I64\n");
    let config = Config::default();
    assert!(r#impl.process(ParseCtx::from(&tokens, &config)).is_err());
}
