use crate::{
    fmt::{format, FormatInput},
    parser::parse_string,
    Config,
};

#[test]
fn do_notation_format_roundtrip() {
    let source = "main = ->\n    result = do\n        x <- first!\n        _ <- second x\n        total = x + 1\n        pure total\n    result\n";
    let parsed = parse_string(source, &Config::default()).unwrap();
    let formatted = format(FormatInput::program(&parsed));
    assert!(formatted.contains("x <- first!"), "{formatted}");
    assert!(formatted.contains("_ <- second x"), "{formatted}");
    let reparsed = parse_string(&formatted, &Config::default()).unwrap();
    assert_eq!(formatted, format(FormatInput::program(&reparsed)));
}

#[test]
fn do_notation_rejects_invalid_statements() {
    for source in [
        "main = -> do\n",
        "main = -> do\n    x <- action!\n",
        "main = -> do\n    x = 1\n",
        "main = -> do\n    (x, y) <- action!\n    pure x\n",
        "main = -> do\n    return action!\n",
        "main = -> do\n    x = action!?\n    pure x\n",
        "main = -> do\n    break\n    pure 1\n",
        "main = -> do\n    if true then return 1\n    pure 1\n",
    ] {
        assert!(
            parse_string(source, &Config::default()).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn do_notation_allows_explicit_function_control_flow() {
    let source = "main = -> do\n    helper = ->\n        return 1\n    pure helper!\n";
    parse_string(source, &Config::default()).unwrap();
}

#[test]
fn do_notation_nested_format_and_comments_roundtrip() {
    let source = "main = -> do\n    // Bind the outer value.\n    x <- first!\n    y <- do\n        z <- second x\n        pure z\n    pure (x + y)\n";
    let module =
        crate::parser::parse_source("example.rk".into(), source, &Config::default()).unwrap();
    let formatted = format(FormatInput::module_with_source(&module, source));
    assert!(
        formatted.contains("// Bind the outer value."),
        "{formatted}"
    );
    assert_eq!(formatted.matches("do").count(), 2, "{formatted}");
    let reparsed =
        crate::parser::parse_source("example.rk".into(), &formatted, &Config::default()).unwrap();
    assert_eq!(
        formatted,
        format(FormatInput::module_with_source(&reparsed, &formatted))
    );
}

#[test]
fn do_notation_allows_local_loop_control_flow() {
    let source =
        "main = -> do\n    action = -> pure 1\n    ignored = loop\n        break\n    action!\n";
    parse_string(source, &Config::default()).unwrap();
}
