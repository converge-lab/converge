//! The cursor-pagination response envelope shared by every list.

use converge_storage::Pagination;
use serde::Serialize;

/// One page of a list: pass `next_cursor` back as `?cursor=` for the next
/// page; `null` means the list is exhausted (or the read was unpaginated).
#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// Wrap a list read: a full page (`len == limit`) points the cursor at
    /// its last item; a short or unlimited read ends the list.
    pub fn new<Id>(items: Vec<T>, page: &Pagination<Id>, id: impl Fn(&T) -> String) -> Self {
        let next_cursor = match page.limit {
            Some(limit) if limit > 0 && items.len() == limit as usize => items.last().map(id),
            _ => None,
        };
        Page { items, next_cursor }
    }
}
