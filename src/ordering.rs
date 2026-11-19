//! Ordering (pure): Waiting On (me › nobody › someone else), then most recently
//! updated. The Scope and sidebar ticket (#9) adds the kind order and Scope
//! filtering here.

use crate::decide::Decision;
use std::cmp::Reverse;

pub fn order(mut decisions: Vec<Decision>) -> Vec<Decision> {
    decisions.sort_by(|a, b| {
        (a.waiting_on, Reverse(a.item.updated_at), &a.item.id).cmp(&(
            b.waiting_on,
            Reverse(b.item.updated_at),
            &b.item.id,
        ))
    });
    decisions
}
