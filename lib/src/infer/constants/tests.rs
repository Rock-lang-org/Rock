use super::*;

#[test]
fn integer_constants_preserve_width_and_report_only_trapping_modes() {
    for ty in [
        Type::I8,
        Type::I16,
        Type::I32,
        Type::I64,
        Type::U8,
        Type::U16,
        Type::U32,
        Type::U64,
    ] {
        let (bits, signed) = width(&ty).unwrap();
        let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
        let max = if signed {
            (1i128 << (bits - 1)) - 1
        } else {
            (1i128 << bits) - 1
        };
        for (op, a, b, wrapped) in [
            (IntegerOp::Add, max, 1, min),
            (IntegerOp::Sub, min, 1, max),
            (IntegerOp::Shl, 1, bits as i128, 1),
            (IntegerOp::Shr, 1, -1, 0),
        ] {
            assert!(integer(
                IntegerIntrinsic {
                    op,
                    mode: IntegerMode::Checked
                },
                &ty,
                a,
                b
            )
            .is_err());
            assert_eq!(
                integer(
                    IntegerIntrinsic {
                        op,
                        mode: IntegerMode::Wrapping
                    },
                    &ty,
                    a,
                    b
                ),
                Ok(Some(Constant::Integer(wrapped)))
            );
            assert_eq!(
                integer(
                    IntegerIntrinsic {
                        op,
                        mode: IntegerMode::Overflowing
                    },
                    &ty,
                    a,
                    b
                ),
                Ok(Some(Constant::Tuple(vec![
                    Constant::Integer(wrapped),
                    Constant::Bool(true)
                ])))
            );
        }
        for op in [IntegerOp::Div, IntegerOp::Rem] {
            for mode in [
                IntegerMode::Checked,
                IntegerMode::Wrapping,
                IntegerMode::Overflowing,
            ] {
                assert!(integer(IntegerIntrinsic { op, mode }, &ty, 1, 0).is_err());
            }
            if signed {
                assert!(integer(
                    IntegerIntrinsic {
                        op,
                        mode: IntegerMode::Checked
                    },
                    &ty,
                    min,
                    -1
                )
                .is_err());
                let expected = if op == IntegerOp::Div { min } else { 0 };
                assert_eq!(
                    integer(
                        IntegerIntrinsic {
                            op,
                            mode: IntegerMode::Wrapping
                        },
                        &ty,
                        min,
                        -1
                    ),
                    Ok(Some(Constant::Integer(expected)))
                );
            }
        }
    }
}

#[test]
fn u64_constant_multiplication_never_overflows_the_host_evaluator() {
    let max = u64::MAX as i128;
    assert_eq!(
        integer(
            IntegerIntrinsic {
                op: IntegerOp::Mul,
                mode: IntegerMode::Overflowing
            },
            &Type::U64,
            max,
            max
        ),
        Ok(Some(Constant::Tuple(vec![
            Constant::Integer(1),
            Constant::Bool(true)
        ])))
    );
    assert!(integer(
        IntegerIntrinsic {
            op: IntegerOp::Mul,
            mode: IntegerMode::Checked
        },
        &Type::U64,
        max,
        max
    )
    .is_err());
}

#[test]
fn arithmetic_summaries_bound_resources_and_select_only_the_taken_policy() {
    let failure = ArithmeticSummary::Intrinsic(
        "IntCheckedAdd".to_string(),
        ArithmeticType::I8,
        vec![
            ArithmeticSummary::Integer(127),
            ArithmeticSummary::Integer(1),
        ],
    );
    let selected = ArithmeticSummary::If(
        Box::new(ArithmeticSummary::Bool(false)),
        Box::new(failure),
        Box::new(ArithmeticSummary::Integer(0)),
    );
    selected.validate(0).unwrap();
    assert_eq!(selected.evaluate(&[]), Ok(Some(Constant::Integer(0))));
    let mut deep = ArithmeticSummary::Integer(0);
    for _ in 0..33 {
        deep = ArithmeticSummary::Ref(Box::new(deep));
    }
    assert!(deep.validate(0).is_err());
    assert_eq!(
        mechanism("I8Neg", &Type::I8, &[Constant::Bool(true)]),
        Ok(None)
    );
}
