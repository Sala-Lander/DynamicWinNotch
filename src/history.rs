//! Clipboard history: newest first, pinned entries stay on top, duplicates move to the front.

use crate::util::single_line;

/// Hard cap on stored characters per entry (keeps RAM bounded even if someone copies a novel).
pub const MAX_CHARS: usize = 4000;
/// Characters kept in the one-line preview shown in the list.
pub const PREVIEW_CHARS: usize = 140;

#[derive(Clone, Debug)]
pub struct Entry {
    pub text: String,
    pub preview: String,
    pub pinned: bool,
}

#[derive(Debug)]
pub struct History {
    items: Vec<Entry>,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self { items: Vec::new(), cap: cap.max(1) }
    }

    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap.max(1);
        self.enforce_cap();
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[Entry] {
        &self.items
    }

    pub fn get(&self, i: usize) -> Option<&Entry> {
        self.items.get(i)
    }

    fn pinned_count(&self) -> usize {
        self.items.iter().take_while(|e| e.pinned).count()
    }

    fn enforce_cap(&mut self) {
        while self.items.len() - self.pinned_count() > self.cap {
            self.items.pop();
        }
    }

    /// Returns true when the visible list changed (a new entry, or an old one moved to the front).
    pub fn push(&mut self, text: &str) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let text: String = if text.chars().count() > MAX_CHARS {
            text.chars().take(MAX_CHARS).collect()
        } else {
            text.to_string()
        };
        let pinned = self.pinned_count();
        if let Some(p) = self.items.iter().position(|e| e.text == text) {
            if self.items[p].pinned || p == pinned {
                return false;
            }
            let e = self.items.remove(p);
            self.items.insert(pinned, e);
            return true;
        }
        let preview = single_line(&text, PREVIEW_CHARS);
        self.items.insert(pinned, Entry { text, preview, pinned: false });
        self.enforce_cap();
        true
    }

    pub fn toggle_pin(&mut self, i: usize) {
        if i >= self.items.len() {
            return;
        }
        let mut e = self.items.remove(i);
        e.pinned = !e.pinned;
        let at = if e.pinned { 0 } else { self.pinned_count() };
        self.items.insert(at, e);
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.items.len() {
            self.items.remove(i);
        }
    }

    pub fn clear_unpinned(&mut self) {
        self.items.retain(|e| e.pinned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(h: &History) -> Vec<&str> {
        h.items().iter().map(|e| e.text.as_str()).collect()
    }

    #[test]
    fn newest_first_and_dedupe() {
        let mut h = History::new(10);
        assert!(h.push("a"));
        assert!(h.push("b"));
        assert!(h.push("c"));
        assert_eq!(texts(&h), ["c", "b", "a"]);
        assert!(h.push("a")); // moves to front
        assert_eq!(texts(&h), ["a", "c", "b"]);
        assert!(!h.push("a")); // already newest
        assert!(!h.push("   \n"));
    }

    #[test]
    fn cap_drops_oldest_unpinned_only() {
        let mut h = History::new(2);
        h.push("keep");
        h.toggle_pin(0);
        h.push("1");
        h.push("2");
        h.push("3");
        assert_eq!(texts(&h), ["keep", "3", "2"]);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn pin_unpin_ordering() {
        let mut h = History::new(10);
        h.push("a");
        h.push("b");
        h.push("c"); // c b a
        h.toggle_pin(2); // pin a -> a c b
        assert_eq!(texts(&h), ["a", "c", "b"]);
        assert!(h.get(0).unwrap().pinned);
        h.push("d"); // goes below pinned: a d c b
        assert_eq!(texts(&h), ["a", "d", "c", "b"]);
        h.toggle_pin(0); // unpin a -> d a c b (newest unpinned)
        assert_eq!(texts(&h), ["a", "d", "c", "b"]);
        assert!(!h.get(0).unwrap().pinned);
    }

    #[test]
    fn clear_keeps_pins() {
        let mut h = History::new(10);
        h.push("a");
        h.push("b");
        h.toggle_pin(1);
        h.clear_unpinned();
        assert_eq!(texts(&h), ["a"]);
    }

    #[test]
    fn long_text_is_truncated_and_previewed() {
        let mut h = History::new(3);
        let big = "x".repeat(MAX_CHARS + 500);
        h.push(&big);
        assert_eq!(h.get(0).unwrap().text.chars().count(), MAX_CHARS);
        assert_eq!(h.get(0).unwrap().preview.chars().count(), PREVIEW_CHARS);
    }
}
