//! Clipboard history: newest first, pinned entries stay on top, duplicates move to the front.
//! Entries are text, links, colours, file lists or images. Images are held compressed and
//! bounded by a byte budget; the oldest unpinned ones are dropped first.

use crate::detect::{classify, Detected};
use crate::util::{fmt_bytes, single_line};
use std::sync::Arc;

/// Hard cap on stored characters per entry (keeps RAM bounded even if someone copies a novel).
pub const MAX_CHARS: usize = 4000;
/// Characters kept in the one-line preview shown in the list.
pub const PREVIEW_CHARS: usize = 140;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// Payload: byte length of the host at the start of the preview.
    Link(usize),
    Color((u8, u8, u8)),
    /// Payload: number of files.
    Files(usize),
    Image,
}

/// A copied image: deflate-compressed RGBA plus a small premultiplied thumbnail for the list.
#[derive(Debug)]
pub struct ImageData {
    pub w: u32,
    pub h: u32,
    pub packed: Vec<u8>,
    pub raw_len: usize,
    pub hash: u64,
    pub thumb_w: u32,
    pub thumb_h: u32,
    pub thumb: Vec<u8>,
}

impl ImageData {
    pub fn stored_bytes(&self) -> usize {
        self.packed.len() + self.thumb.len()
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    /// What is copied back (empty for images).
    pub text: String,
    pub preview: String,
    pub pinned: bool,
    pub image: Option<Arc<ImageData>>,
}

#[derive(Debug)]
pub struct History {
    items: Vec<Entry>,
    cap: usize,
    budget: usize,
}

fn clip_text(text: &str) -> String {
    if text.chars().count() > MAX_CHARS {
        text.chars().take(MAX_CHARS).collect()
    } else {
        text.to_string()
    }
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self { items: Vec::new(), cap: cap.max(1), budget: 64 * 1024 * 1024 }
    }

    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap.max(1);
        self.enforce_cap();
    }

    /// Byte budget for the stored images.
    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
        self.enforce_budget();
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

    /// The most recently added unpinned entry (what a "Copied ..." peek describes).
    pub fn newest(&self) -> Option<&Entry> {
        self.items.get(self.pinned_count())
    }

    pub fn image_bytes(&self) -> usize {
        self.items.iter().filter_map(|e| e.image.as_ref()).map(|i| i.stored_bytes()).sum()
    }

    fn pinned_count(&self) -> usize {
        self.items.iter().take_while(|e| e.pinned).count()
    }

    fn enforce_cap(&mut self) {
        while self.items.len() - self.pinned_count() > self.cap {
            self.items.pop();
        }
    }

    /// Drop the oldest unpinned images until the budget holds. The newest unpinned entry is
    /// never dropped, so a single huge screenshot still shows up.
    fn enforce_budget(&mut self) {
        while self.image_bytes() > self.budget {
            let keep_from = self.pinned_count() + 1;
            let victim = (keep_from..self.items.len()).rev().find(|&i| self.items[i].image.is_some() && !self.items[i].pinned);
            match victim {
                Some(i) => {
                    self.items.remove(i);
                }
                None => break,
            }
        }
    }

    /// Move entry `i` to the front (below the pinned block). Returns its new index.
    pub fn bump(&mut self, i: usize) -> usize {
        let pinned = self.pinned_count();
        if i >= self.items.len() || self.items[i].pinned || i == pinned {
            return i.min(self.items.len().saturating_sub(1));
        }
        let e = self.items.remove(i);
        self.items.insert(pinned, e);
        pinned
    }

    fn insert_new(&mut self, e: Entry) {
        let pinned = self.pinned_count();
        self.items.insert(pinned, e);
        self.enforce_cap();
        self.enforce_budget();
    }

    /// Returns true when the visible list changed (a new entry, or an old one moved to the front).
    fn push_entry(&mut self, text: String, kind: Kind, preview: String) -> bool {
        let is_files = matches!(kind, Kind::Files(_));
        let existing = self
            .items
            .iter()
            .position(|e| e.image.is_none() && e.text == text && matches!(e.kind, Kind::Files(_)) == is_files);
        if let Some(p) = existing {
            let pinned = self.pinned_count();
            if self.items[p].pinned || p == pinned {
                return false;
            }
            self.bump(p);
            return true;
        }
        self.insert_new(Entry { kind, text, preview, pinned: false, image: None });
        true
    }

    /// Add copied text. Links and colour values are recognised automatically.
    pub fn push(&mut self, text: &str) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let text = clip_text(text);
        let (kind, preview) = match classify(&text) {
            Detected::Link { display, host_len } => {
                let preview = single_line(&display, PREVIEW_CHARS);
                let host_len = host_len.min(preview.len());
                (Kind::Link(host_len), preview)
            }
            Detected::Color(c) => (Kind::Color(c), single_line(&text, 40)),
            Detected::Text => (Kind::Text, single_line(&text, PREVIEW_CHARS)),
        };
        self.push_entry(text, kind, preview)
    }

    /// Add a list of copied files (full paths). Clicking the row copies the paths as text.
    pub fn push_files(&mut self, paths: &[String]) -> bool {
        if paths.is_empty() {
            return false;
        }
        let text = clip_text(&paths.join("\n"));
        let name = |p: &str| p.rsplit(|c: char| c == '\\' || c == '/').next().unwrap_or(p).to_string();
        let preview = if paths.len() == 1 {
            name(&paths[0])
        } else {
            format!("{} +{} more", name(&paths[0]), paths.len() - 1)
        };
        let preview = single_line(&preview, PREVIEW_CHARS);
        self.push_entry(text, Kind::Files(paths.len()), preview)
    }

    /// Add a copied image. The same pixels copied again just move to the front.
    pub fn push_image(&mut self, img: ImageData) -> bool {
        let existing = self
            .items
            .iter()
            .position(|e| e.image.as_ref().is_some_and(|i| i.hash == img.hash && i.w == img.w && i.h == img.h));
        if let Some(p) = existing {
            let pinned = self.pinned_count();
            if self.items[p].pinned || p == pinned {
                return false;
            }
            self.bump(p);
            return true;
        }
        let preview = format!("Image \u{b7} {}\u{d7}{} \u{b7} {}", img.w, img.h, fmt_bytes(img.raw_len));
        self.insert_new(Entry { kind: Kind::Image, text: String::new(), preview, pinned: false, image: Some(Arc::new(img)) });
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

    /// Absolute indices of entries matching `query` (case-insensitive substring of the preview
    /// text, or of the full text for anything that is not an image), in display order. An empty
    /// query matches everything.
    pub fn filter(&self, query: &str) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return (0..self.items.len()).collect();
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, e)| e.preview.to_lowercase().contains(&q) || e.text.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
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

    fn img(hash: u64, packed: usize) -> ImageData {
        ImageData {
            w: 100,
            h: 50,
            packed: vec![0; packed],
            raw_len: 100 * 50 * 4,
            hash,
            thumb_w: 2,
            thumb_h: 1,
            thumb: vec![0; 8],
        }
    }

    #[test]
    fn kinds_are_detected() {
        let mut h = History::new(10);
        h.push("plain words");
        h.push("https://github.com/rust-lang/rust");
        h.push("#FF8800");
        let k: Vec<Kind> = h.items().iter().map(|e| e.kind).collect();
        assert_eq!(k, [Kind::Color((255, 136, 0)), Kind::Link(10), Kind::Text]);
        assert_eq!(h.get(1).unwrap().preview, "github.com/rust-lang/rust");
        // The original text is what gets copied back.
        assert_eq!(h.get(1).unwrap().text, "https://github.com/rust-lang/rust");
    }

    #[test]
    fn files_entry() {
        let mut h = History::new(10);
        assert!(!h.push_files(&[]));
        assert!(h.push_files(&["C:\\a\\report.pdf".to_string(), "C:\\a\\b.txt".to_string()]));
        let e = h.get(0).unwrap();
        assert_eq!(e.kind, Kind::Files(2));
        assert_eq!(e.preview, "report.pdf +1 more");
        assert_eq!(e.text, "C:\\a\\report.pdf\nC:\\a\\b.txt");
        assert!(h.push_files(&["/tmp/x.png".to_string()]));
        assert_eq!(h.get(0).unwrap().preview, "x.png");
        // The same paths copied again do not duplicate.
        assert!(h.push_files(&["C:\\a\\report.pdf".to_string(), "C:\\a\\b.txt".to_string()]));
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn images_dedupe_by_hash_and_move_to_front() {
        let mut h = History::new(10);
        assert!(h.push_image(img(1, 10)));
        assert!(h.push_image(img(2, 10)));
        h.push("text");
        assert!(h.push_image(img(2, 10))); // copied again: moves to the front
        assert_eq!(h.len(), 3);
        assert!(h.push_image(img(1, 10)));
        assert_eq!(h.get(0).unwrap().image.as_ref().unwrap().hash, 1);
        assert_eq!(h.get(1).unwrap().image.as_ref().unwrap().hash, 2);
        assert!(!h.push_image(img(1, 10))); // already newest
        assert_eq!(h.get(0).unwrap().preview, "Image \u{b7} 100\u{d7}50 \u{b7} 20 KB");
        assert_eq!(h.get(0).unwrap().text, "");
    }

    #[test]
    fn budget_evicts_oldest_unpinned_images_only() {
        let mut h = History::new(50);
        h.set_budget(1000);
        h.push_image(img(1, 392)); // 400 bytes stored each (392 + 8 thumb)
        h.toggle_pin(0); // pinned images are never evicted
        h.push_image(img(2, 392));
        h.push("text stays");
        h.push_image(img(3, 392));
        assert_eq!(h.image_bytes(), 1200 - 400); // 1200 > 1000: image 2 (oldest unpinned) went
        let hashes: Vec<u64> = h.items().iter().filter_map(|e| e.image.as_ref().map(|i| i.hash)).collect();
        assert_eq!(hashes, [1, 3]);
        assert!(h.items().iter().any(|e| e.text == "text stays"));
    }

    #[test]
    fn newest_image_survives_even_if_over_budget() {
        let mut h = History::new(10);
        h.set_budget(100);
        h.push_image(img(1, 5000));
        assert_eq!(h.len(), 1);
        h.push_image(img(2, 5000));
        assert_eq!(h.len(), 1);
        assert_eq!(h.get(0).unwrap().image.as_ref().unwrap().hash, 2);
        h.set_budget(1 << 30);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn bump_returns_new_index() {
        let mut h = History::new(10);
        h.push("a");
        h.push("b");
        h.push("c"); // c b a
        assert_eq!(h.bump(2), 0);
        assert_eq!(texts(&h), ["a", "c", "b"]);
        h.toggle_pin(1); // pin c -> c a b
        assert_eq!(h.bump(2), 1); // lands below the pinned block
        assert_eq!(texts(&h), ["c", "b", "a"]);
        assert_eq!(h.bump(0), 0); // pinned stays
        assert_eq!(h.bump(99), 2);
    }

    #[test]
    fn filter_matches_preview_and_text_case_insensitively() {
        let mut h = History::new(10);
        h.push("Hello World");
        h.push("https://github.com/rust-lang/rust");
        h.push("#FF8800");
        // Newest push ends up at index 0: colour, then link, then the text.
        assert_eq!(h.filter(""), [0, 1, 2]);
        assert_eq!(h.filter("world"), [2]);
        assert_eq!(h.filter("GITHUB"), [1]);
        assert_eq!(h.filter("ff8800"), [0]);
        assert!(h.filter("nope").is_empty());
    }

    #[test]
    fn cap_counts_images_too() {
        let mut h = History::new(2);
        h.push_image(img(1, 1));
        h.push("a");
        h.push("b");
        assert_eq!(h.len(), 2);
        assert!(h.items().iter().all(|e| e.image.is_none()));
    }
}
