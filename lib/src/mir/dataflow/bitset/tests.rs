use super::*;

use crate::ids::LoanId;
use crate::mir::Local;

#[test]
fn bitset_tracks_typed_ids_and_joins() {
    let mut left = BitSet::<LoanId>::new();
    let mut right = BitSet::<LoanId>::new();

    assert!(left.insert(LoanId(1)));
    assert!(!left.insert(LoanId(1)));
    assert!(right.insert(LoanId(2)));

    assert!(left.contains(LoanId(1)));
    assert!(!left.contains(LoanId(2)));
    assert!(left.join(&right));
    assert!(left.contains(LoanId(2)));
    assert_eq!(left.iter().collect::<Vec<_>>(), vec![LoanId(1), LoanId(2)]);
}

#[test]
fn local_set_tracks_mir_locals_and_removes() {
    let mut set = LocalSet::new();

    assert!(set.insert(Local(3)));
    assert!(set.contains(Local(3)));
    assert!(set.remove(Local(3)));
    assert!(!set.contains(Local(3)));
    assert!(set.is_empty());
}
