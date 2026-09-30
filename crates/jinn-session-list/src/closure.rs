//! Descendant closure over parent links, keyed by session id.
//!
//! The tree a set of parent links describes is a different question from the
//! tree a set of *loaded* sessions describes, and both are asked in this
//! codebase: a sidebar asks the second to decide what to draw, an actor asks
//! the first to decide what to dispose of. This module answers the first,
//! because that one does not depend on what happens to be loaded.

use std::collections::{HashMap, HashSet, VecDeque};

use jinn_core_types::SessionId;

/// Every session reachable from `root` through parent links, root first.
///
/// Breadth-first, and cycle-safe: a link cycle yields each session once and
/// terminates. `parent_of` need not contain `root` — an unknown root is its
/// own closure.
///
/// Order is load-bearing for callers that treat the head as the root, which
/// `Breadth-first, root first` guarantees without the caller re-deriving it.
#[must_use]
pub fn descendant_closure(
    root: &SessionId,
    parent_of: &HashMap<SessionId, Option<SessionId>>,
) -> Vec<SessionId> {
    let mut children_of: HashMap<SessionId, Vec<SessionId>> = HashMap::new();
    for (id, parent) in parent_of {
        if let Some(parent_id) = parent {
            children_of
                .entry(parent_id.clone())
                .or_default()
                .push(id.clone());
        }
    }

    let mut closure = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([root.clone()]);
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        closure.push(id.clone());
        if let Some(children) = children_of.get(&id) {
            queue.extend(children.iter().cloned());
        }
    }
    closure
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::descendant_closure;
    use jinn_core_types::SessionId;
    use std::collections::HashMap;

    /// A chain of three ids, distinct by construction.
    fn trio() -> (SessionId, SessionId, SessionId) {
        (SessionId::new(), SessionId::new(), SessionId::new())
    }

    /// A three-deep chain: root -> a -> b.
    fn chain(
        root: &SessionId,
        a: &SessionId,
        b: &SessionId,
    ) -> HashMap<SessionId, Option<SessionId>> {
        HashMap::from([
            (a.clone(), Some(root.clone())),
            (b.clone(), Some(a.clone())),
            (root.clone(), None),
        ])
    }

    #[rstest::rstest]
    fn a_chain_yields_every_descendant() {
        // Given a parent map forming root -> a -> b.
        let (root, a, b) = trio();
        let map = chain(&root, &a, &b);

        // When the closure is taken.
        let closure = descendant_closure(&root, &map);

        // Then it holds all three, root first.
        assert_eq!(
            closure.len(),
            3,
            "the whole chain is reachable: {closure:?}"
        );
        assert_eq!(
            closure.first(),
            Some(&root),
            "the root is the head, which callers rely on"
        );
    }

    #[rstest::rstest]
    fn a_leaf_yields_only_itself() {
        // Given a parent map with a childless root.
        let root = SessionId::new();
        let map = HashMap::from([(root.clone(), None)]);

        // When the closure is taken.
        let closure = descendant_closure(&root, &map);

        // Then it is the root alone.
        assert_eq!(closure, vec![root]);
    }

    #[rstest::rstest]
    fn an_unknown_root_yields_itself() {
        // Given a parent map that does not mention the root at all.
        let map: HashMap<SessionId, Option<SessionId>> = HashMap::new();
        let root = SessionId::new();

        // When the closure is taken.
        let closure = descendant_closure(&root, &map);

        // Then the root is its own closure, rather than an error or empty.
        assert_eq!(closure, vec![root]);
    }

    #[rstest::rstest]
    fn a_link_cycle_terminates_and_yields_each_session_once() {
        // Given two sessions that name each other as parent.
        let a = SessionId::new();
        let b = SessionId::new();
        let map = HashMap::from([(a.clone(), Some(b.clone())), (b.clone(), Some(a.clone()))]);

        // When the closure is taken.
        let closure = descendant_closure(&a, &map);

        // Then it terminates and holds each of them once.
        assert_eq!(closure, vec![a, b], "a cycle cannot loop forever or repeat");
    }
}
