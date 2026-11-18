//! Title/artist normalization ported from lyrics.js (cleanTitle, cleanArtist, titleMatches).

use std::collections::HashSet;
use std::fmt::Write;

/// Drops video noise like "(Official Music Video)", a trailing "| ..." and a leading "Artist - ".
pub fn clean_title(title: &str) -> String {
    let mut t = strip_noise(title);
    if let Some(i) = t.find('|') {
        t.truncate(i);
    }
    let t = t.trim();
    if t.contains(" - ") {
        let parts = split_dash(t);
        if parts.len() == 2 && !parts[1].is_empty() {
            return parts[1].trim().to_string();
        }
    }
    t.to_string()
}

/// Drops YouTube's "Artist - Topic" suffix.
pub fn clean_artist(artist: &str) -> String {
    let stripped = artist
        .len()
        .checked_sub(5)
        .filter(|&i| artist.is_char_boundary(i) && artist[i..].eq_ignore_ascii_case("topic"))
        .and_then(|i| artist[..i].trim_end().strip_suffix('-'));
    stripped.unwrap_or(artist).trim().to_string()
}

/// First artist of "A, B", "A & B", "A feat. B", "A ft B".
pub fn first_artist(artist: &str) -> &str {
    let low = artist.to_ascii_lowercase();
    let b = low.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b',' | b'&' => return artist[..i].trim(),
            c if c.is_ascii_whitespace() => {
                let j = skip_ws(b, i);
                for w in [&b"feat"[..], b"ft"] {
                    if b[j..].starts_with(w) {
                        let mut k = j + w.len();
                        if b.get(k) == Some(&b'.') {
                            k += 1;
                        }
                        if skip_ws(b, k) > k {
                            return artist[..i].trim();
                        }
                    }
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    artist.trim()
}

/// Loose match: equal, contained, or at least half the words shared.
pub fn title_matches(candidate: &str, target: &str) -> bool {
    let norm = |s: &str| {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (c, t) = (norm(candidate), norm(target));
    if c.is_empty() || t.is_empty() {
        return false;
    }
    if c == t || c.contains(&t) || t.contains(&c) {
        return true;
    }
    let wc: HashSet<&str> = c.split(' ').collect();
    let wt: HashSet<&str> = t.split(' ').collect();
    let shared = wt.iter().filter(|w| wc.contains(*w)).count();
    shared as f64 >= wc.len().min(wt.len()) as f64 * 0.5
}

/// encodeURIComponent.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn skip_ws(b: &[u8], from: usize) -> usize {
    from + b.get(from..).unwrap_or(&[]).iter().take_while(|c| c.is_ascii_whitespace()).count()
}

/// `str.split(/\s+-\s+/)`.
fn split_dash(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let (mut parts, mut last, mut i) = (Vec::new(), 0, 0);
    while i < b.len() {
        if !b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let j = skip_ws(b, i);
        if b.get(j) == Some(&b'-') {
            let k = skip_ws(b, j + 1);
            if k > j + 1 {
                parts.push(&s[last..i]);
                last = k;
                i = k;
                continue;
            }
        }
        i = j;
    }
    parts.push(&s[last..]);
    parts
}

/// Removes every `\s*[([]...keyword...[)\]]` group (the JS NOISE regex).
fn strip_noise(s: &str) -> String {
    let b = s.as_bytes();
    let low = s.to_ascii_lowercase();
    let (mut out, mut last, mut i) = (String::with_capacity(s.len()), 0, 0);
    while i < b.len() {
        if b[i] == b'(' || b[i] == b'[' {
            let close = b[i + 1..].iter().position(|&c| c == b')' || c == b']').map(|p| i + 1 + p);
            if let Some(close) = close.filter(|&c| has_noise_word(&low.as_bytes()[i + 1..c])) {
                let mut start = i;
                while start > last && b[start - 1].is_ascii_whitespace() {
                    start -= 1;
                }
                out.push_str(&s[last..start]);
                last = close + 1;
                i = last;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[last..]);
    out
}

/// `\b(official(\s+music)?\s+video|official\s+audio|lyrics?|audio|video|mv|m\/v|visuali[sz]er|hd|4k|remaster(ed)?|live(\s+version)?)\b`
/// on lowercased ASCII bytes.
fn has_noise_word(c: &[u8]) -> bool {
    let word = |i: usize| c.get(i).is_some_and(|x| x.is_ascii_alphanumeric() || *x == b'_');
    let at = |k: usize, w: &[u8]| c.get(k..).is_some_and(|r| r.starts_with(w));
    let after_ws = |k: usize| Some(skip_ws(c, k)).filter(|&e| e > k);
    (0..c.len()).filter(|&i| i == 0 || !word(i - 1)).any(|i| {
        let mut ends = Vec::new();
        for w in [
            &b"lyrics"[..], b"lyric", b"audio", b"video", b"mv", b"m/v", b"visualizer", b"visualiser", b"hd",
            b"4k", b"remastered", b"remaster", b"live",
        ] {
            if at(i, w) {
                ends.push(i + w.len());
            }
        }
        if at(i, b"official") {
            if let Some(k) = after_ws(i + 8) {
                if at(k, b"video") || at(k, b"audio") {
                    ends.push(k + 5);
                }
                if let Some(n) = Some(k + 5).filter(|_| at(k, b"music")).and_then(after_ws) {
                    if at(n, b"video") {
                        ends.push(n + 5);
                    }
                }
            }
        }
        if let Some(k) = Some(i + 4).filter(|_| at(i, b"live")).and_then(after_ws) {
            if at(k, b"version") {
                ends.push(k + 7);
            }
        }
        ends.into_iter().any(|e| !word(e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_titles() {
        assert_eq!(clean_title("Artist - Song (Official Music Video)"), "Song");
        assert_eq!(clean_title("Song [Lyrics] | Some Label"), "Song");
        assert_eq!(clean_title("Live Forever (Remastered 2009)"), "Live Forever");
        assert_eq!(clean_title("Song (Live at Wembley)"), "Song");
        assert_eq!(clean_title("Holiday (Lyric Video)"), "Holiday");
        // Keywords only count as whole words inside brackets.
        assert_eq!(clean_title("Song (HDR)"), "Song (HDR)");
        assert_eq!(clean_title("Song (feat. X)"), "Song (feat. X)");
        assert_eq!(clean_title("Video Killed the Radio Star"), "Video Killed the Radio Star");
        // More than one " - " is ambiguous; keep it.
        assert_eq!(clean_title("A - B - C"), "A - B - C");
    }

    #[test]
    fn cleans_artists() {
        assert_eq!(clean_artist("Adele - Topic"), "Adele");
        assert_eq!(clean_artist("Topic"), "Topic");
        assert_eq!(first_artist("A feat. B"), "A");
        assert_eq!(first_artist("A, B & C"), "A");
        assert_eq!(first_artist("A ft B"), "A");
        assert_eq!(first_artist("The Featherweights"), "The Featherweights");
    }

    #[test]
    fn matches_titles() {
        assert!(title_matches("Song (Remix)", "song"));
        assert!(title_matches("너의 의미", "너의 의미 (Feat. 김창완)"));
        assert!(!title_matches("Other Thing", "Song"));
        assert!(!title_matches("", "Song"));
        assert_eq!(enc("a b/ü"), "a%20b%2F%C3%BC");
    }
}
