use super::{LayeredMap, FOLD_THRESHOLD};

#[test]
fn clone_is_independent_of_later_writes() {
    let mut parent = LayeredMap::default();
    for key in 0..FOLD_THRESHOLD * 3 {
        parent.insert(key, key);
    }
    let mut probe = parent.clone();
    probe.insert(1, 100);
    probe.insert(10_000, 7);
    assert_eq!(parent.get(&1), Some(&1));
    assert!(!parent.contains_key(&10_000));
    assert_eq!(probe[&1], 100);
    assert_eq!(probe[&10_000], 7);
}

#[test]
fn shared_base_is_never_folded_into() {
    let mut parent = LayeredMap::default();
    let probe = parent.clone();
    for key in 0..FOLD_THRESHOLD * 2 {
        parent.insert(key, key);
    }
    assert!(
        probe.iter().next().is_none(),
        "writes must not leak into a clone"
    );
    drop(probe);
    parent.insert(usize::MAX, 0);
    assert_eq!(parent.iter().count(), FOLD_THRESHOLD * 2 + 1);
}

#[test]
fn iteration_and_entries_see_overrides_once() {
    let mut map: LayeredMap<u32, Vec<u32>> = (0..4).map(|key| (key, vec![key])).collect();
    map.entry_or_default(2).push(20);
    map.entry_or_default(9).push(90);
    map.insert_if_absent(3, vec![]);
    let mut entries = map.iter().map(|(k, v)| (*k, v.clone())).collect::<Vec<_>>();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            (0, vec![0]),
            (1, vec![1]),
            (2, vec![2, 20]),
            (3, vec![3]),
            (9, vec![90])
        ]
    );
}
