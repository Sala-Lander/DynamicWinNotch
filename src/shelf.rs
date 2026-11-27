//! The Shelf: a temporary place for files. Drop a file on the island, leave it there, drag it out
//! into another app later.
//!
//! This module is the pure part (items, limits, expiry, tile geometry, persistence, copying), so
//! it is unit-tested on any platform. The OLE drag-and-drop glue lives in `win.rs`, the drawing
//! in `ui.rs`, the wiring in `app.rs`.

use crate::config::ShelfExpire;
use crate::layout::Rect;
use crate::util::fmt_bytes;
use std::path::{Path, PathBuf};

/// Most items on the shelf; one more pushes the oldest off.
pub const MAX_ITEMS: usize = 24;

pub const TILE_W: f32 = 76.0;
pub const TILE_H: f32 = 84.0;
pub const TILE_GAP: f32 = 8.0;
/// Size of the "remove" button in a tile's top-right corner.
pub const X_SIZE: f32 = 16.0;
/// Tiles that fit the 416 px content area without clipping.
pub const VISIBLE_TILES: usize = 5;
/// Pointer travel (logical px) with the button held before a press becomes a drag-out.
pub const DRAG_THRESHOLD: f32 = 5.0;

// ------------------------------------------------------------------------------------- kinds

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Pdf,
    Image,
    Text,
    Sheet,
    Doc,
    Slides,
    Video,
    Audio,
    Archive,
    Code,
    App,
    Other,
}

impl Kind {
    /// Up to four letters, drawn on the tile's icon.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Folder => "DIR",
            Kind::Pdf => "PDF",
            Kind::Image => "IMG",
            Kind::Text => "TXT",
            Kind::Sheet => "XLS",
            Kind::Doc => "DOC",
            Kind::Slides => "PPT",
            Kind::Video => "VID",
            Kind::Audio => "AUD",
            Kind::Archive => "ZIP",
            Kind::Code => "CODE",
            Kind::App => "EXE",
            Kind::Other => "FILE",
        }
    }

    pub fn color(self) -> (u8, u8, u8) {
        match self {
            Kind::Folder => (240, 200, 110),
            Kind::Pdf => (255, 107, 107),
            Kind::Image => (107, 213, 160),
            Kind::Text => (201, 207, 216),
            Kind::Sheet => (123, 224, 123),
            Kind::Doc => (110, 168, 255),
            Kind::Slides => (255, 160, 90),
            Kind::Video => (183, 148, 255),
            Kind::Audio => (255, 130, 200),
            Kind::Archive => (255, 200, 87),
            Kind::Code => (120, 220, 230),
            Kind::App => (200, 200, 255),
            Kind::Other => (170, 176, 190),
        }
    }
}

/// File type of `name`, from its extension.
pub fn kind_of(name: &str, is_dir: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    let ext = match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() => e.to_ascii_lowercase(),
        _ => return Kind::Other,
    };
    match ext.as_str() {
        "pdf" => Kind::Pdf,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff" | "heic" => Kind::Image,
        "txt" | "md" | "log" | "rtf" | "ini" | "cfg" => Kind::Text,
        "xls" | "xlsx" | "csv" | "ods" | "tsv" => Kind::Sheet,
        "doc" | "docx" | "odt" | "pages" => Kind::Doc,
        "ppt" | "pptx" | "odp" | "key" => Kind::Slides,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" => Kind::Video,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" => Kind::Audio,
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "iso" => Kind::Archive,
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "cs" | "java" | "go" | "json" | "html" | "css" | "toml"
        | "yml" | "yaml" | "xml" | "sh" | "ps1" | "bat" | "sql" => Kind::Code,
        "exe" | "msi" | "lnk" => Kind::App,
        _ => Kind::Other,
    }
}

/// Last path component (both `\` and `/` separate), so it works for any platform's paths.
pub fn file_name(path: &str) -> &str {
    let t = path.trim_end_matches(['\\', '/']);
    t.rsplit(['\\', '/']).next().unwrap_or(t)
}

// ------------------------------------------------------------------------------------- items

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub dir: bool,
    /// Unix seconds when it was put on the shelf.
    pub added: u64,
    /// `path` is a copy made by the shelf (mode "copies"): it is deleted together with the item.
    pub owned: bool,
}

impl Item {
    pub fn new(path: &str, size: u64, dir: bool, added: u64, owned: bool) -> Item {
        Item { path: path.to_string(), name: file_name(path).to_string(), size, dir, added, owned }
    }

    pub fn kind(&self) -> Kind {
        kind_of(&self.name, self.dir)
    }
}

/// What `Shelf::add` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Added {
    /// Added; carries the oldest item when the shelf was full and made room.
    New(Option<Item>),
    /// The same path is already on the shelf.
    Duplicate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shelf {
    items: Vec<Item>,
}

impl Shelf {
    pub fn new() -> Shelf {
        Shelf::default()
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&Item> {
        self.items.get(i)
    }

    pub fn add(&mut self, item: Item) -> Added {
        if self.items.iter().any(|x| x.path.eq_ignore_ascii_case(&item.path)) {
            return Added::Duplicate;
        }
        let evicted = if self.items.len() >= MAX_ITEMS { Some(self.items.remove(0)) } else { None };
        self.items.push(item);
        Added::New(evicted)
    }

    pub fn remove(&mut self, i: usize) -> Option<Item> {
        (i < self.items.len()).then(|| self.items.remove(i))
    }

    /// Empty the shelf; returns what was on it (so owned copies can be deleted).
    pub fn clear(&mut self) -> Vec<Item> {
        std::mem::take(&mut self.items)
    }

    pub fn total_size(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    /// Remove (and return) every item older than the expiry setting.
    pub fn take_expired(&mut self, now: u64, e: ShelfExpire) -> Vec<Item> {
        let Some(limit) = expiry_secs(e) else { return Vec::new() };
        let (gone, keep): (Vec<Item>, Vec<Item>) =
            std::mem::take(&mut self.items).into_iter().partition(|i| now.saturating_sub(i.added) >= limit);
        self.items = keep;
        gone
    }

    /// Remove (and return) items for which `exists` says the file is gone.
    pub fn take_missing(&mut self, exists: impl Fn(&Item) -> bool) -> Vec<Item> {
        let (keep, gone): (Vec<Item>, Vec<Item>) = std::mem::take(&mut self.items).into_iter().partition(|i| exists(i));
        self.items = keep;
        gone
    }

    /// When the next item expires (unix seconds), if expiry is time based and the shelf is not empty.
    pub fn next_expiry(&self, e: ShelfExpire) -> Option<u64> {
        let limit = expiry_secs(e)?;
        self.items.iter().map(|i| i.added + limit).min()
    }

    /// Footer text: "3 items · 4.2 MB · clears in 59 min".
    pub fn footer(&self, e: ShelfExpire, now: u64) -> String {
        let n = self.items.len();
        let mut s = format!("{n} {} \u{b7} {}", if n == 1 { "item" } else { "items" }, fmt_bytes(self.total_size() as usize));
        match e {
            ShelfExpire::Never => {}
            ShelfExpire::OnQuit => s.push_str(" \u{b7} clears on quit"),
            _ => {
                if let Some(t) = self.next_expiry(e) {
                    s.push_str(&format!(" \u{b7} clears in {}", fmt_left(t.saturating_sub(now))));
                }
            }
        }
        s
    }
}

/// Lifetime of an item in seconds (`None` = no time limit).
pub fn expiry_secs(e: ShelfExpire) -> Option<u64> {
    match e {
        ShelfExpire::Hour => Some(3600),
        ShelfExpire::Day => Some(86_400),
        ShelfExpire::Never | ShelfExpire::OnQuit => None,
    }
}

/// "<1 min", "59 min", "5 h", "2 d".
pub fn fmt_left(secs: u64) -> String {
    match secs {
        0..=59 => "<1 min".to_string(),
        60..=3599 => format!("{} min", secs / 60),
        3600..=86_399 => format!("{} h", secs / 3600),
        _ => format!("{} d", secs / 86_400),
    }
}

// ------------------------------------------------------------------------------- persistence

/// One line per item: `added<TAB>size<TAB>flags<TAB>path` (flags: `d` folder, `o` owned copy, `-` none).
pub fn to_text(s: &Shelf) -> String {
    let mut out = String::new();
    for i in &s.items {
        let mut flags = String::new();
        if i.dir {
            flags.push('d');
        }
        if i.owned {
            flags.push('o');
        }
        if flags.is_empty() {
            flags.push('-');
        }
        out.push_str(&format!("{}\t{}\t{}\t{}\n", i.added, i.size, flags, i.path));
    }
    out
}

pub fn from_text(text: &str) -> Shelf {
    let mut s = Shelf::new();
    for line in text.lines() {
        let mut p = line.splitn(4, '\t');
        let (Some(a), Some(sz), Some(fl), Some(path)) = (p.next(), p.next(), p.next(), p.next()) else { continue };
        let (Ok(added), Ok(size)) = (a.parse::<u64>(), sz.parse::<u64>()) else { continue };
        if path.is_empty() {
            continue;
        }
        s.add(Item::new(path, size, fl.contains('d'), added, fl.contains('o')));
    }
    s
}

// --------------------------------------------------------------------------- files on disk

/// Item for an existing path (size and folder flag from the file system).
pub fn scan(path: &Path, now: u64) -> Option<Item> {
    let md = std::fs::metadata(path).ok()?;
    let p = path.to_str()?;
    Some(Item::new(p, if md.is_dir() { 0 } else { md.len() }, md.is_dir(), now, false))
}

/// `name`, or `name (2)`, `name (3)`... when `taken` says the name is in use.
pub fn unique_name(taken: &dyn Fn(&str) -> bool, name: &str) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (2..10_000)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|c| !taken(c))
        .unwrap_or_else(|| format!("{stem} (copy){ext}"))
}

/// Copy a file into `dir` (created if needed) under a free name; returns the copy's path.
pub fn copy_into(dir: &Path, src: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let free = unique_name(&|n| dir.join(n).exists(), name);
    let dst = dir.join(free);
    std::fs::copy(src, &dst)?;
    Ok(dst)
}

/// Delete the copy behind an owned item. Never touches anything outside `dir`, so a corrupted
/// list file cannot make the shelf delete somebody's real files.
pub fn remove_owned(item: &Item, dir: &Path) -> bool {
    if !item.owned || item.dir {
        return false;
    }
    let p = Path::new(&item.path);
    p.starts_with(dir) && !item.path.contains("..") && std::fs::remove_file(p).is_ok()
}

// ------------------------------------------------------------------------------------ layout

/// Tile `i` of the strip; `scroll` is in tiles (fractional while animating).
pub fn tile_rect(content: Rect, i: usize, scroll: f32) -> Rect {
    Rect::new(content.x + (i as f32 - scroll) * (TILE_W + TILE_GAP), content.y, TILE_W, TILE_H)
}

/// The remove button inside a tile.
pub fn x_rect(tile: Rect) -> Rect {
    Rect::new(tile.right() - X_SIZE - 3.0, tile.y + 3.0, X_SIZE, X_SIZE)
}

/// How far the strip can scroll, in tiles.
pub fn max_scroll(n: usize) -> f32 {
    n.saturating_sub(VISIBLE_TILES) as f32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileHit {
    /// The tile itself: click opens, press-and-move drags it out.
    Item(usize),
    /// The tile's remove button.
    Remove(usize),
}

/// What is under (x, y) on the Shelf page with `n` items.
pub fn tile_hit(content: Rect, n: usize, scroll: f32, x: f32, y: f32) -> Option<TileHit> {
    if !content.contains(x, y) {
        return None;
    }
    (0..n).find_map(|i| {
        let t = tile_rect(content, i, scroll);
        if !t.contains(x, y) {
            None
        } else if x_rect(t).inflate(2.0).contains(x, y) {
            Some(TileHit::Remove(i))
        } else {
            Some(TileHit::Item(i))
        }
    })
}

/// Has the pointer moved far enough from where the button went down to start a drag?
pub fn drag_started(down: (f32, f32), now: (f32, f32)) -> bool {
    let (dx, dy) = (now.0 - down.0, now.1 - down.1);
    dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD
}

// -------------------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn item(p: &str, added: u64) -> Item {
        Item::new(p, 1000, false, added, false)
    }

    fn content() -> Rect {
        Rect::new(18.0, 46.0, 416.0, 112.0)
    }

    #[test]
    fn names_and_kinds() {
        assert_eq!(file_name("C:\\Users\\me\\report.pdf"), "report.pdf");
        assert_eq!(file_name("C:\\Users\\me\\Folder\\"), "Folder");
        assert_eq!(file_name("/tmp/a.b.txt"), "a.b.txt");
        assert_eq!(kind_of("Report.PDF", false), Kind::Pdf);
        assert_eq!(kind_of("a.tar.gz", false), Kind::Archive);
        assert_eq!(kind_of(".gitignore", false), Kind::Other);
        assert_eq!(kind_of("noext", false), Kind::Other);
        assert_eq!(kind_of("Pictures", true), Kind::Folder);
        assert_eq!(kind_of("a.rs", false).label(), "CODE");
        for k in [Kind::Folder, Kind::Pdf, Kind::Other, Kind::App] {
            assert!(k.label().len() <= 4);
        }
    }

    #[test]
    fn add_dedupes_and_evicts_oldest() {
        let mut s = Shelf::new();
        assert_eq!(s.add(item("C:\\a.txt", 1)), Added::New(None));
        assert_eq!(s.add(item("c:\\A.TXT", 2)), Added::Duplicate);
        assert_eq!(s.len(), 1);
        for i in 1..MAX_ITEMS {
            assert_eq!(s.add(item(&format!("C:\\f{i}.txt"), 10 + i as u64)), Added::New(None));
        }
        assert_eq!(s.len(), MAX_ITEMS);
        match s.add(item("C:\\new.txt", 99)) {
            Added::New(Some(old)) => assert_eq!(old.path, "C:\\a.txt"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.len(), MAX_ITEMS);
        assert_eq!(s.items().last().unwrap().name, "new.txt");
    }

    #[test]
    fn remove_and_clear() {
        let mut s = Shelf::new();
        s.add(item("a", 1));
        s.add(item("b", 2));
        assert_eq!(s.remove(5), None);
        assert_eq!(s.remove(0).unwrap().path, "a");
        assert_eq!(s.clear().len(), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn expiry_removes_only_old_items() {
        let mut s = Shelf::new();
        s.add(item("old", 1000));
        s.add(item("new", 4000));
        assert!(s.take_expired(5000, ShelfExpire::Never).is_empty());
        assert!(s.take_expired(5000, ShelfExpire::OnQuit).is_empty());
        assert_eq!(s.next_expiry(ShelfExpire::Hour), Some(4600));
        let gone = s.take_expired(4600, ShelfExpire::Hour);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].path, "old");
        assert_eq!(s.len(), 1);
        assert_eq!(s.take_expired(4000 + 86_400, ShelfExpire::Day).len(), 1);
        assert_eq!(s.next_expiry(ShelfExpire::Day), None);
    }

    #[test]
    fn missing_files_are_dropped() {
        let mut s = Shelf::new();
        s.add(item("keep", 1));
        s.add(item("gone", 2));
        let gone = s.take_missing(|i| i.path == "keep");
        assert_eq!(gone.len(), 1);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn footer_texts() {
        let mut s = Shelf::new();
        s.add(Item::new("a", 2048, false, 1000, false));
        assert_eq!(s.footer(ShelfExpire::Never, 1000), format!("1 item \u{b7} {}", fmt_bytes(2048)));
        assert!(s.footer(ShelfExpire::Hour, 1000 + 600).ends_with("clears in 50 min"));
        assert!(s.footer(ShelfExpire::OnQuit, 1000).ends_with("clears on quit"));
        s.add(item("b", 1000));
        assert!(s.footer(ShelfExpire::Day, 1000).starts_with("2 items"));
        assert_eq!(fmt_left(0), "<1 min");
        assert_eq!(fmt_left(59), "<1 min");
        assert_eq!(fmt_left(3599), "59 min");
        assert_eq!(fmt_left(3600 * 5), "5 h");
        assert_eq!(fmt_left(86_400 * 2), "2 d");
    }

    #[test]
    fn text_round_trip_and_garbage() {
        let mut s = Shelf::new();
        s.add(Item::new("C:\\a b\\x.txt", 5, false, 10, false));
        s.add(Item::new("C:\\a b\\dir", 0, true, 11, false));
        s.add(Item::new("C:\\data\\shelf\\c.png", 9, false, 12, true));
        let t = to_text(&s);
        assert_eq!(from_text(&t), s);
        let junk = format!("{t}nonsense\n1\t2\n\tx\ty\tz\n5\t5\t-\t\n");
        assert_eq!(from_text(&junk), s);
        assert!(from_text("").is_empty());
    }

    #[test]
    fn unique_names() {
        let taken = |n: &str| n == "a.txt" || n == "a (2).txt" || n == "noext";
        assert_eq!(unique_name(&taken, "b.txt"), "b.txt");
        assert_eq!(unique_name(&taken, "a.txt"), "a (3).txt");
        assert_eq!(unique_name(&taken, "noext"), "noext (2)");
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("notch-shelf-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copies_get_free_names_and_only_owned_copies_are_deleted() {
        let root = temp_dir("copy");
        let src = root.join("doc.txt");
        std::fs::write(&src, b"hello").unwrap();
        let shelf_dir = root.join("shelf");
        let a = copy_into(&shelf_dir, &src).unwrap();
        let b = copy_into(&shelf_dir, &src).unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&b).unwrap(), b"hello");

        let it = scan(&a, 5).unwrap();
        assert_eq!(it.size, 5);
        assert!(!it.dir);
        let owned = Item { owned: true, ..it.clone() };
        // a link item is never deleted, nor is an owned item outside the shelf folder
        assert!(!remove_owned(&it, &shelf_dir));
        let outside = Item::new(src.to_str().unwrap(), 5, false, 1, true);
        assert!(!remove_owned(&outside, &shelf_dir));
        assert!(src.exists());
        assert!(remove_owned(&owned, &shelf_dir));
        assert!(!a.exists());
        assert!(scan(&root.join("nope"), 1).is_none());
        assert!(scan(&root, 1).unwrap().dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tiles_lay_out_in_a_scrolling_strip() {
        let c = content();
        let t0 = tile_rect(c, 0, 0.0);
        assert_eq!((t0.x, t0.y, t0.w, t0.h), (c.x, c.y, TILE_W, TILE_H));
        assert!(tile_rect(c, VISIBLE_TILES - 1, 0.0).right() <= c.right());
        assert!(tile_rect(c, 1, 1.0).x == c.x);
        assert!(t0.bottom() <= c.bottom());
        assert_eq!(max_scroll(3), 0.0);
        assert_eq!(max_scroll(8), 3.0);
        let x = x_rect(t0);
        assert!(x.x >= t0.x && x.right() <= t0.right() && x.y >= t0.y);
    }

    #[test]
    fn tile_hits() {
        let c = content();
        let t1 = tile_rect(c, 1, 0.0);
        assert_eq!(tile_hit(c, 3, 0.0, t1.cx(), t1.cy()), Some(TileHit::Item(1)));
        let x = x_rect(t1);
        assert_eq!(tile_hit(c, 3, 0.0, x.cx(), x.cy()), Some(TileHit::Remove(1)));
        // gap between tiles, past the last tile, outside the content area
        assert_eq!(tile_hit(c, 3, 0.0, t1.x - 4.0, t1.cy()), None);
        let t3 = tile_rect(c, 3, 0.0);
        assert_eq!(tile_hit(c, 3, 0.0, t3.cx(), t3.cy()), None);
        assert_eq!(tile_hit(c, 3, 0.0, 2.0, 2.0), None);
        // scrolling moves the hit areas with the tiles
        let t = tile_rect(c, 5, 2.0);
        assert_eq!(tile_hit(c, 8, 2.0, t.cx(), t.cy()), Some(TileHit::Item(5)));
    }

    #[test]
    fn drag_needs_real_movement() {
        assert!(!drag_started((10.0, 10.0), (12.0, 11.0)));
        assert!(drag_started((10.0, 10.0), (16.0, 10.0)));
        assert!(drag_started((10.0, 10.0), (10.0, 4.0)));
    }
}
