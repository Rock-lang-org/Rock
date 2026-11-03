use super::*;

use crate::mir::BasicBlockId;

#[test]
fn location_is_typed_and_hashable() {
    let loc = Location::new(BasicBlockId(2), StatementIndex(5));
    let mut map = std::collections::HashMap::new();

    map.insert(loc, "loan");

    assert_eq!(map.get(&loc), Some(&"loan"));
    assert_eq!(loc.block, BasicBlockId(2));
    assert_eq!(loc.statement, StatementIndex(5));
}
