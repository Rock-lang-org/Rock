use super::Kind;

#[test]
fn kind_display_is_right_associative() {
    assert_eq!(
        Kind::arrow(Kind::Type, Kind::arrow(Kind::Type, Kind::Type)).to_string(),
        "Type -> Type -> Type"
    );
    assert_eq!(
        Kind::arrow(Kind::arrow(Kind::Type, Kind::Type), Kind::Type).to_string(),
        "(Type -> Type) -> Type"
    );
}
