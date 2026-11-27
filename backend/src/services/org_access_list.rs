//! Set semantics shared by revisioned organization access lists.
use serde_json::json;
use std::collections::BTreeSet;

pub(crate) struct ListChange {
    added: usize,
    removed: usize,
    inherited_before: bool,
    inherited_after: bool,
    self_added: bool,
}

impl ListChange {
    pub(crate) fn between(
        before: &Option<Vec<String>>,
        after: &Option<Vec<String>>,
        actor: &str,
    ) -> Self {
        let prior: BTreeSet<_> = before.iter().flatten().map(String::as_str).collect();
        let next: BTreeSet<_> = after.iter().flatten().map(String::as_str).collect();
        Self {
            added: next.difference(&prior).count(),
            removed: prior.difference(&next).count(),
            inherited_before: before.is_none(),
            inherited_after: after.is_none(),
            self_added: next.contains(actor) && !prior.contains(actor),
        }
    }
    pub(crate) fn changed(&self) -> bool {
        self.inherited_after != self.inherited_before || self.added > 0 || self.removed > 0
    }
    pub(crate) fn narrowed(&self) -> bool {
        !self.inherited_after && (self.inherited_before || self.removed > 0)
    }
    pub(crate) fn audit(&self) -> serde_json::Value {
        json!({"added":self.added,"removed":self.removed,"inherited_before":self.inherited_before,
            "inherited_after":self.inherited_after,"self_added":self.self_added})
    }
}
