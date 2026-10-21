#[path = "../../tests/support/macro_fixture.rs"]
mod macro_fixture;
#[path = "../../tests/support/macro_syntax.rs"]
mod macro_syntax;
#[path = "../../tests/support/macro_types.rs"]
mod macro_types;
#[path = "../../tests/support/macro_generics.rs"]
mod macro_generics;
#[path = "../../tests/support/macro_structs.rs"]
mod macro_structs;
#[path = "../../tests/support/macro_enums.rs"]
mod macro_enums;
#[path = "../../tests/support/macro_emission.rs"]
mod macro_emission;

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

#[test]
fn macro_c_05_shared_generic_fixtures_have_expected_outcomes() {
    macro_generics::verify_oracle();
}

#[test]
#[ignore = "native Rock generic syntax conformance; run cargo macro-acceptance"]
fn macro_acceptance_syntax_generics() {
    macro_generics::verify(&SdkCase::new("generics", "").config);
}

#[test]
fn macro_c_06_shared_struct_fixtures_have_expected_outcomes() {
    macro_structs::verify_oracle();
}

#[test]
#[ignore = "native Rock derive-input struct conformance; run cargo macro-acceptance"]
fn macro_acceptance_syntax_structs() {
    macro_structs::verify(&SdkCase::new("structs", "").config);
}

#[test]
fn macro_c_07_shared_enum_fixtures_have_expected_outcomes() {
    macro_enums::verify_oracle();
}

#[test]
#[ignore = "native Rock derive-input enum conformance; run cargo macro-acceptance"]
fn macro_acceptance_syntax_enums() {
    macro_enums::verify(&SdkCase::new("enums", "").config);
}

#[test]
fn macro_c_08_emission_grouping_oracle() {
    macro_emission::verify_oracle();
}

#[test]
#[ignore = "native Rock emission and hygienic impl conformance; run cargo macro-acceptance"]
fn macro_acceptance_syntax_emission() {
    macro_emission::verify(&SdkCase::new("emission", "").config);
}
