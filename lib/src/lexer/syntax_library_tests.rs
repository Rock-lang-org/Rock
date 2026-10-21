#[path = "../../tests/support/macro_fixture.rs"]
mod macro_fixture;
#[path = "../../tests/support/macro_syntax.rs"]
mod macro_syntax;
#[path = "../../tests/support/macro_types.rs"]
mod macro_types;

use macro_fixture::SdkCase;

#[test]
fn macro_c_03_shared_lexical_group_fixtures_have_expected_outcomes() {
    macro_syntax::verify_oracle();
}

#[test]
#[ignore = "native Rock syntax conformance; run cargo macro-acceptance"]
fn macro_acceptance_syntax_cursor_and_groups() {
    macro_syntax::verify(&SdkCase::new("syntax", "").config);
}

#[test]
fn macro_c_04_shared_type_fixtures_have_expected_outcomes() {
    macro_types::verify_oracle();
}

#[test]
#[ignore = "native Rock type syntax conformance; run cargo macro-acceptance"]
fn macro_acceptance_syntax_types() {
    macro_types::verify(&SdkCase::new("types", "").config);
}
