//! Undo/redo: snapshots of the document taken around each message.

use std::mem::{Discriminant, discriminant};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{App, Message};
use crate::model::{Group, Rule};

const CAP: usize = 100;

#[derive(Debug, Clone)]
pub(super) struct Snapshot {
    rules: Vec<Rule>,
    groups: Vec<Group>,
    epoch: u64,
}

#[derive(Debug, Default)]
pub(super) struct Stack {
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    last_key: Option<(Key, Option<usize>)>,
    /// Bumped when a message replaces the document wholesale (load, undo, redo): that
    /// message isn't recorded as an edit.
    epoch: u64,
}

/// Messages with equal keys on the same rule are one undo step (typing in a field).
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Key {
    Field(Discriminant<Message>, Option<usize>),
    Unique(u64),
}

pub(super) fn key(msg: &Message) -> Option<Key> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let field = |i| Some(Key::Field(discriminant(msg), i));
    match msg {
        Message::Select(_)
        | Message::Search(_)
        | Message::Tab(_)
        | Message::Refresh
        | Message::Undo
        | Message::Redo => None,
        Message::Name(_)
        | Message::SizeW(_)
        | Message::SizeH(_)
        | Message::MoveX(_)
        | Message::MoveY(_)
        | Message::Workspace(_)
        | Message::Monitor(_)
        | Message::Group(_) => field(None),
        Message::MatchText(i, _) | Message::PropKey(i, _) | Message::PropValue(i, _) => {
            field(Some(*i))
        }
        _ => Some(Key::Unique(COUNTER.fetch_add(1, Ordering::Relaxed))),
    }
}

impl Stack {
    pub(super) fn reset(&mut self) {
        let epoch = self.epoch + 1;
        *self = Self {
            epoch,
            ..Self::default()
        };
    }

    /// Record `before` as an undo step after a message that `changed` the document.
    fn push(&mut self, before: Snapshot, changed: bool, key: Option<(Key, Option<usize>)>) {
        if !changed {
            return;
        }
        if key.is_some() && key == self.last_key {
            return;
        }
        self.past.push(before);
        if self.past.len() > CAP {
            self.past.remove(0);
        }
        self.future.clear();
        self.last_key = key;
    }
}

impl App {
    pub(super) fn snapshot(&self) -> Snapshot {
        Snapshot {
            rules: self.rules.clone(),
            groups: self.groups.clone(),
            epoch: self.undo.epoch,
        }
    }

    pub(super) fn record(&mut self, before: Snapshot, key: Option<Key>) {
        let changed = before.epoch == self.undo.epoch
            && (before.rules != self.rules || before.groups != self.groups);
        let key = key.map(|k| (k, self.selected));
        self.undo.push(before, changed, key);
    }

    pub(super) fn undo_step(&mut self) {
        if let Some(snap) = self.undo.past.pop() {
            let now = self.snapshot();
            self.undo.future.push(now);
            self.restore(snap);
        }
    }

    pub(super) fn redo_step(&mut self) {
        if let Some(snap) = self.undo.future.pop() {
            let now = self.snapshot();
            self.undo.past.push(now);
            self.restore(snap);
        }
    }

    fn restore(&mut self, snap: Snapshot) {
        self.rules = snap.rules;
        self.groups = snap.groups;
        self.undo.epoch += 1;
        self.undo.last_key = None;
        self.dirty = self.rules != self.saved_rules;
        self.select(self.selected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str) -> Snapshot {
        Snapshot {
            rules: vec![Rule {
                name: name.into(),
                ..Default::default()
            }],
            groups: Vec::new(),
            epoch: 0,
        }
    }

    fn field(msg: &Message, sel: usize) -> Option<(Key, Option<usize>)> {
        key(msg).map(|k| (k, Some(sel)))
    }

    #[test]
    fn typing_in_one_field_is_one_step() {
        let mut s = Stack::default();
        s.push(snap("a"), true, field(&Message::Name("ab".into()), 0));
        s.push(snap("ab"), true, field(&Message::Name("abc".into()), 0));
        assert_eq!(s.past.len(), 1);
    }

    #[test]
    fn different_fields_are_separate_steps() {
        let mut s = Stack::default();
        s.push(snap("a"), true, field(&Message::Name("ab".into()), 0));
        s.push(
            snap("ab"),
            true,
            field(&Message::PropValue(0, "1".into()), 0),
        );
        assert_eq!(s.past.len(), 2);
    }

    #[test]
    fn same_field_on_another_rule_is_a_new_step() {
        let mut s = Stack::default();
        s.push(snap("a"), true, field(&Message::Name("ab".into()), 0));
        s.push(snap("b"), true, field(&Message::Name("bc".into()), 1));
        assert_eq!(s.past.len(), 2);
    }

    #[test]
    fn unchanged_documents_and_new_edits() {
        let mut s = Stack::default();
        s.push(snap("a"), false, field(&Message::Name("a".into()), 0));
        assert!(s.past.is_empty());
        s.push(snap("a"), true, key(&Message::Duplicate).map(|k| (k, None)));
        s.future.push(snap("x"));
        s.push(snap("b"), true, key(&Message::Duplicate).map(|k| (k, None)));
        assert_eq!(s.past.len(), 2);
        assert!(s.future.is_empty());
    }
}
