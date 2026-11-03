use super::*;

#[test]
fn integer_identities_round_trip_without_source_type_names() {
    for op in [
        IntegerOp::Add,
        IntegerOp::Sub,
        IntegerOp::Mul,
        IntegerOp::Div,
        IntegerOp::Rem,
        IntegerOp::Neg,
        IntegerOp::Shl,
        IntegerOp::Shr,
    ] {
        for mode in [
            IntegerMode::Checked,
            IntegerMode::Wrapping,
            IntegerMode::Overflowing,
        ] {
            let intrinsic = IntegerIntrinsic { op, mode };
            assert_eq!(
                IntegerIntrinsic::from_name(intrinsic.as_str()),
                Some(intrinsic)
            );
            let bytes = bincode::serialize(&intrinsic).unwrap();
            assert_eq!(
                bincode::deserialize::<IntegerIntrinsic>(&bytes).unwrap(),
                intrinsic
            );
        }
    }
    assert!(IntegerIntrinsic::from_name("IntUncheckedAdd").is_none());
}
