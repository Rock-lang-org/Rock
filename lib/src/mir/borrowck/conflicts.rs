use crate::mir::{Place, Projection};

pub fn places_conflict(a: &Place, b: &Place) -> bool {
    if a.local != b.local {
        return false;
    }

    for (lhs, rhs) in a.projection.iter().zip(&b.projection) {
        if lhs != rhs {
            return match (lhs, rhs) {
                (Projection::Field { index: x, .. }, Projection::Field { index: y, .. })
                    if x != y =>
                {
                    false
                }
                _ => true,
            };
        }
    }

    true
}

#[cfg(test)]
mod tests;
