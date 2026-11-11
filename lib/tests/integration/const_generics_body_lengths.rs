use crate::support::*;

#[test]
fn test_const_generics_body_local_repeat_and_for_loop_specialize_each_length() {
    let output = compile_and_run(r#"
visit: [I64; N] -> I64
visit = template ->
    local: [I64; N] = [7; N]
    for value in local
        value.println!
    N

main = !->
    visit [1, 2] .println!
    visit [1, 2, 3] .println!
    empty: [I64; 0] = []
    visit empty .println!
"#);
    assert_eq!(output.trim(), "7\n7\n2\n7\n7\n7\n3\n0");
}
