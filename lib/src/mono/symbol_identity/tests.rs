use super::*;

impl Monomorphizer {
    pub(crate) fn register_test_dependency_identity(&mut self, crate_id: crate::ids::CrateId) {
        self.identity_context.dependencies.insert(
            crate_id,
            ProductCrateIdentity::local("test-dependency".to_string()),
        );
    }
}

fn encoded_type(mono: &mut Monomorphizer, ty: Type) -> String {
    let id = mono.type_context.intern_type(&ty);
    let mut output = String::new();
    mono.write_type_id(&mut output, id);
    output
}

#[test]
fn const_value_identity_is_distinct_from_char_and_length_delimited() {
    let mut mono = Monomorphizer::new();
    let char_then_one = encoded_type(&mut mono, Type::Tuple(vec![Type::Char, Type::ConstValue(1)]));
    let const_eleven = encoded_type(&mut mono, Type::ConstValue(11));
    let const_one = encoded_type(&mut mono, Type::ConstValue(1));
    let char_ty = encoded_type(&mut mono, Type::Char);

    assert_ne!(const_one, const_eleven);
    assert_ne!(char_ty, const_one);
    assert!(!char_then_one.contains("c1"), "{char_then_one}");
    assert!(!const_one.contains('c'), "{const_one}");
}
