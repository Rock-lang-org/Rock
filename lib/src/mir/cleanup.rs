//! Lexical cleanup scheduling shared by concrete and descriptor-backed bodies.

pub(crate) struct CleanupScopes<L> {
    scopes: Vec<Vec<L>>,
}

impl<L> Default for CleanupScopes<L> {
    fn default() -> Self {
        Self { scopes: Vec::new() }
    }
}

impl<L: Copy + PartialEq> CleanupScopes<L> {
    pub(crate) fn enter(&mut self, locals: Vec<L>) {
        self.scopes.push(locals);
    }

    pub(crate) fn register(&mut self, local: L) {
        if let Some(scope) = self.scopes.last_mut() {
            if !scope.contains(&local) {
                scope.push(local);
            }
        }
    }

    pub(crate) fn leave(&mut self) -> Vec<L> {
        self.scopes.pop().unwrap_or_default()
    }

    pub(crate) fn depth(&self) -> usize {
        self.scopes.len()
    }

    pub(crate) fn current(&self) -> Option<&Vec<L>> {
        self.scopes.last()
    }

    /// Abrupt exits include every active statement and lexical scope.
    pub(crate) fn active_from(&self, depth: usize) -> Vec<L> {
        self.scopes.iter().skip(depth).flatten().copied().collect()
    }
}

/// Both representations destroy locals in reverse registration order.
pub(crate) fn cleanup_order<L: Copy>(locals: &[L]) -> impl Iterator<Item = L> + '_ {
    locals.iter().rev().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_and_abrupt_exits_use_the_same_nested_cleanup_order() {
        let mut scopes = CleanupScopes::default();
        scopes.enter(vec![0]);
        scopes.register(1);
        scopes.enter(vec![2]);
        scopes.register(3);
        scopes.enter(vec![4]);
        assert_eq!(
            cleanup_order(&scopes.active_from(1)).collect::<Vec<_>>(),
            [4, 3, 2]
        );
        assert_eq!(cleanup_order(&scopes.leave()).collect::<Vec<_>>(), [4]);
        assert_eq!(cleanup_order(&scopes.leave()).collect::<Vec<_>>(), [3, 2]);
        assert_eq!(cleanup_order(&scopes.leave()).collect::<Vec<_>>(), [1, 0]);
        assert_eq!(scopes.depth(), 0);
    }
}
