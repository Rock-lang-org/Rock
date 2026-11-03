use crate::support::*;

#[test]
fn test_hash_trait_scalar_and_str() {
    let output = compile_and_run(
        r#"
main = ->
    42.hash! .println!
    (-42).hash! .println!
    true.hash! .println!
    false.hash! .println!
    'A'.hash! .println!
    "abc".hash! .println!
    "abc".hash! .println!
    "abd".hash! .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "-42");
    assert_eq!(lines[2], "1");
    assert_eq!(lines[3], "0");
    assert_eq!(lines[4], "65");
    assert_eq!(lines[5], lines[6]);
    assert_ne!(lines[5], lines[7]);
}

#[test]
fn test_hash_map_insert_get_len_and_contains() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.len! .println!
    map.insert 10, 100
    map.insert 20, 200
    map.len! .println!
    ten = 10
    twenty = 20
    thirty = 30
    map.contains_key &ten .println!
    map.contains_key &thirty .println!

    match (map.get &ten)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &twenty)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &thirty)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["0", "2", "true", "false", "100", "200", "-1"],);
}

#[test]
fn test_std_hash_map_rejects_zero_sized_values_on_insert() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

main = ->
    mut map = HashMap::new!
    map.insert 1, Empty
    0
"#,
    );

    assert!(!success, "HashMap::insert should reject zero-sized values");
}

#[test]
fn test_std_hash_map_rejects_zero_sized_keys_on_insert() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

impl Eq for Empty

impl PartialEq Empty for Empty
    @== = other -> true
    @!= = other -> false

impl Hash for Empty
    @hash = -> 0

main = ->
    mut map = HashMap::new!
    map.insert Empty, 1
    0
"#,
    );

    assert!(!success, "HashMap::insert should reject zero-sized keys");
}

#[test]
fn test_hash_map_insert_overwrites_without_growing_len() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert 1, 10
    map.insert 1, 99
    map.len! .println!
    one = 1

    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "99"]);
}

#[test]
fn test_hash_map_handles_collisions_and_growth() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert 1, 10
    map.insert 9, 90
    map.insert 17, 170
    map.insert 25, 250
    map.insert 33, 330
    map.insert 41, 410
    map.insert 49, 490

    map.len! .println!
    one = 1
    nine = 9
    forty_nine = 49

    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!
    match (map.get &nine)
        Option::Some val => *val .println!
        Option::None => -1 .println!
    match (map.get &forty_nine)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["7", "10", "90", "490"]);
}

#[test]
fn test_hash_map_distinguishes_unequal_keys_with_same_hash() {
    let output = compile_and_run(
        r#"
struct BadKey
    < id: I64

impl Eq for BadKey

impl PartialEq BadKey for BadKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for BadKey
    @hash = -> 1

main = ->
    mut map = HashMap::new!
    map.insert (BadKey
        id: 1), 10
    map.insert (BadKey
        id: 2), 20
    map.len! .println!

    one = BadKey
        id: 1
    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    two = BadKey
        id: 2
    match (map.get &two)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "10", "20"]);
}

#[test]
fn test_hash_map_private_storage_fields_are_rejected() {
    compile_should_fail(
        r#"
main = ->
    map = HashMap::new!
    x = map.raw_len
    0
"#,
        "Field 'raw_len' of struct 'HashMap' is private",
    );
}

#[test]
fn test_hash_map_str_keys() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert "red", 1
    map.insert "blue", 2
    map.insert "red", 3
    map.len! .println!
    red = "red"
    green = "green"

    match (map.get &red)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &green)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "3", "-1"]);
}

#[test]
fn test_std_hash_map_drops_keys_and_values() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 2
        drop_id: 102), (TrackedValue
        id: 20)
    0
"#,
    );

    let mut lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    lines.sort();
    assert_eq!(lines, vec![10, 20, 101, 102]);
}

#[test]
fn test_std_hash_map_overwrite_drops_old_value_and_unused_key_once() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 1
        drop_id: 102), (TrackedValue
        id: 20)
    0
"#,
    );

    let lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    assert_eq!(lines, vec![10, 102, 20, 101]);
}

#[test]
fn test_std_hash_map_get_and_contains_borrow_probe_key_without_consuming() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)

    get_probe = TrackedKey
        id: 1
        drop_id: 201
    match (map.get &get_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    get_probe.id.println!

    contains_probe = TrackedKey
        id: 1
        drop_id: 202
    map.contains_key &contains_probe .println!
    contains_probe.id.println!

    missing_probe = TrackedKey
        id: 2
        drop_id: 203
    match (map.get &missing_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    missing_probe.id.println!

    second_get_probe = TrackedKey
        id: 1
        drop_id: 204
    match (map.get &second_get_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    second_get_probe.id.println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["10", "1", "true", "1", "-1", "2", "10", "1", "204", "203", "202", "201", "10", "101"]
    );
}

#[test]
fn test_std_hash_map_growth_drops_moved_keys_and_values_once() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey

impl PartialEq TrackedKey for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 2
        drop_id: 102), (TrackedValue
        id: 20)
    map.insert (TrackedKey
        id: 3
        drop_id: 103), (TrackedValue
        id: 30)
    map.insert (TrackedKey
        id: 4
        drop_id: 104), (TrackedValue
        id: 40)
    map.insert (TrackedKey
        id: 5
        drop_id: 105), (TrackedValue
        id: 50)
    map.insert (TrackedKey
        id: 6
        drop_id: 106), (TrackedValue
        id: 60)
    map.insert (TrackedKey
        id: 7
        drop_id: 107), (TrackedValue
        id: 70)
    0
"#,
    );

    let mut lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        vec![10, 20, 30, 40, 50, 60, 70, 101, 102, 103, 104, 105, 106, 107],
    );
}
