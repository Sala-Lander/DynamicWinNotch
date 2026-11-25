//! Recognises what a piece of copied text is: a web link, a colour value or plain text.
//! Pure string logic, so it is unit-tested.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detected {
    Text,
    /// `display` is the link without scheme / `www.` / trailing slash; `host_len` is the byte
    /// length of the host part at the start of `display`.
    Link { display: String, host_len: usize },
    Color((u8, u8, u8)),
}

pub fn classify(text: &str) -> Detected {
    let t = text.trim();
    if let Some(c) = parse_color(t) {
        return Detected::Color(c);
    }
    if let Some((display, host_len)) = parse_link(t) {
        return Detected::Link { display, host_len };
    }
    Detected::Text
}

fn hex_pair(h: &str, i: usize) -> Option<u8> {
    u8::from_str_radix(h.get(i..i + 2)?, 16).ok()
}

/// `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)`, `rgba(r, g, b, a)`.
pub fn parse_color(t: &str) -> Option<(u8, u8, u8)> {
    let t = t.trim();
    if let Some(h) = t.strip_prefix('#') {
        if !h.is_ascii() || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        return match h.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
                Some((d(0)?, d(1)?, d(2)?))
            }
            6 | 8 => Some((hex_pair(h, 0)?, hex_pair(h, 2)?, hex_pair(h, 4)?)),
            _ => None,
        };
    }
    let lower = t.to_ascii_lowercase();
    let inner = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))?;
    let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let ch = |s: &str| s.parse::<u16>().ok().filter(|&v| v <= 255).map(|v| v as u8);
    Some((ch(parts[0])?, ch(parts[1])?, ch(parts[2])?))
}

fn parse_link(t: &str) -> Option<(String, usize)> {
    if t.is_empty() || t.len() > 2048 || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &t[8..]
    } else if lower.starts_with("http://") {
        &t[7..]
    } else if lower.starts_with("ftp://") {
        &t[6..]
    } else if lower.starts_with("www.") {
        t
    } else {
        return None;
    };
    let rest = rest.strip_prefix("www.").or_else(|| rest.strip_prefix("WWW.")).unwrap_or(rest);
    let host_len = rest.find(|c: char| matches!(c, '/' | '?' | '#')).unwrap_or(rest.len());
    let host = &rest[..host_len];
    // A host needs a dot (example.com), a port or to be localhost; "https://" alone is not a link.
    let plausible = host.len() >= 3 && (host.contains('.') || host.contains(':') || host.eq_ignore_ascii_case("localhost"));
    if !plausible {
        return None;
    }
    let display = rest.trim_end_matches('/').to_string();
    let host_len = host_len.min(display.len());
    Some((display, host_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(parse_color("#fff"), Some((255, 255, 255)));
        assert_eq!(parse_color(" #7AA8FF "), Some((0x7A, 0xA8, 0xFF)));
        assert_eq!(parse_color("#11223344"), Some((0x11, 0x22, 0x33)));
        assert_eq!(parse_color("rgb(10, 20,30)"), Some((10, 20, 30)));
        assert_eq!(parse_color("RGBA(1,2,3,0.5)"), Some((1, 2, 3)));
        assert_eq!(parse_color("rgb(256,0,0)"), None);
        assert_eq!(parse_color("rgb(1,2)"), None);
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#ggg"), None);
        assert_eq!(parse_color("decade"), None); // bare hex-looking words are just text
        assert_eq!(parse_color("#\u{e9}\u{e9}\u{e9}"), None); // non-ASCII never panics on slicing
    }

    #[test]
    fn links() {
        assert_eq!(
            classify("https://github.com/rust-lang/rust/"),
            Detected::Link { display: "github.com/rust-lang/rust".into(), host_len: 10 }
        );
        assert_eq!(
            classify("  http://www.example.com?q=1 "),
            Detected::Link { display: "example.com?q=1".into(), host_len: 11 }
        );
        assert_eq!(classify("www.rust-lang.org"), Detected::Link { display: "rust-lang.org".into(), host_len: 13 });
        assert_eq!(
            classify("http://localhost:8080/x"),
            Detected::Link { display: "localhost:8080/x".into(), host_len: 14 }
        );
    }

    #[test]
    fn not_links() {
        assert_eq!(classify("https://"), Detected::Text);
        assert_eq!(classify("see https://example.com now"), Detected::Text);
        assert_eq!(classify("example.com"), Detected::Text);
        assert_eq!(classify("https://nodots"), Detected::Text);
        assert_eq!(classify("just some text"), Detected::Text);
        assert_eq!(classify(""), Detected::Text);
    }

    #[test]
    fn colour_wins_over_text() {
        assert_eq!(classify("#FF8800"), Detected::Color((255, 136, 0)));
    }
}
