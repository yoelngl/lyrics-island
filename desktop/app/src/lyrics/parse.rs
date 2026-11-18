//! LRC (incl. enhanced word tags) and TTML parsers producing model lines.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::model::{Line, Word};

/// Word end not given by the source; resolved in `finalize`.
const NO_END: u64 = u64::MAX;

/// Credit-line keywords (KuGou, QQ, Netease put "Lyrics by：…", "作曲：…" in the timed lyrics).
const CREDIT_KEYS: &[&str] = &[
    "lyrics", "lyricist", "written", "writer", "compos", "music", "arrang", "produc", "mix", "master", "record",
    "engineer", "vocal", "guitar", "bass", "drum", "keyboard", "piano", "string", "publish", "label", "词", "曲",
    "编", "制作", "监制", "混音", "母带", "和声", "合声", "吉他", "贝斯", "鼓", "录音", "出品", "发行", "企划",
    "演唱", "原唱",
];

/// Parses LRC text; None when it has no timed line with text.
pub fn parse_lrc(lrc: &str) -> Option<Vec<Line>> {
    let mut out = Vec::new();
    for raw in lrc.lines() {
        let (mut times, mut body_start, mut i) = (Vec::new(), 0, 0);
        while let Some(off) = raw[i..].find('[') {
            let at = i + off;
            match stamp_at(&raw[at..], b'[', b']') {
                Some((t, len)) => {
                    times.push(t);
                    i = at + len;
                    body_start = i;
                }
                None => i = at + 1,
            }
        }
        if times.is_empty() {
            continue;
        }
        let body = &raw[body_start..];

        // Enhanced LRC: each <mm:ss.xx> starts a word; one after the final word ends the line.
        let (mut lead, mut tags) = (String::new(), Vec::<(u64, String)>::new());
        let mut i = 0;
        while let Some(off) = body[i..].find('<') {
            let at = i + off;
            let target = tags.last_mut().map_or(&mut lead, |t| &mut t.1);
            match stamp_at(&body[at..], b'<', b'>') {
                Some((t, len)) => {
                    target.push_str(&body[i..at]);
                    tags.push((t, String::new()));
                    i = at + len;
                }
                None => {
                    target.push_str(&body[i..=at]);
                    i = at + 1;
                }
            }
        }
        tags.last_mut().map_or(&mut lead, |t| &mut t.1).push_str(&body[i..]);
        let line_end = if tags.last().is_some_and(|t| t.1.trim().is_empty()) { tags.pop().map(|t| t.0) } else { None };

        let (text, words, base) = match tags.first() {
            None => (collapse(&lead), Vec::new(), None),
            Some(&(first, _)) => {
                // Word tags are absolute; shift them for repeated stamps relative to the nearest one.
                let base = *times.iter().min_by_key(|t| t.abs_diff(first)).unwrap();
                let mut raw_words = Vec::with_capacity(tags.len() + 1);
                if !lead.trim().is_empty() {
                    raw_words.push(Word { start_ms: base.min(first), end_ms: first, text: lead });
                }
                for k in 0..tags.len() {
                    let end_ms = tags.get(k + 1).map(|t| t.0).or(line_end).unwrap_or(NO_END);
                    raw_words.push(Word { start_ms: tags[k].0, end_ms, text: std::mem::take(&mut tags[k].1) });
                }
                let (text, words) = normalize(raw_words);
                (text, words, Some(base))
            }
        };
        for &t in &times {
            let delta = base.map_or(0, |b| t as i64 - b as i64);
            let shift = |ms: u64| if ms == NO_END { ms } else { (ms as i64 + delta).max(0) as u64 };
            let words = words
                .iter()
                .map(|w| Word { start_ms: shift(w.start_ms), end_ms: shift(w.end_ms), text: w.text.clone() })
                .collect();
            let end_ms = line_end.map(shift).filter(|&e| e > t);
            out.push(Line { start_ms: t, end_ms, text: text.clone(), words });
        }
    }
    finalize(out)
}

/// Parses TTML (lrc.red, Better Lyrics): `<p begin end>` lines with optional timed `<span>` words.
pub fn parse_ttml(ttml: &str) -> Option<Vec<Line>> {
    struct Para {
        begin: Option<u64>,
        end: Option<u64>,
        lead: String,
        words: Vec<Word>,
        depth: usize,
        skip_from: Option<usize>,
    }
    let mut reader = Reader::from_str(ttml);
    let (mut out, mut para) = (Vec::new(), None::<Para>);
    let push = |p: &mut Para, s: &str| {
        if p.skip_from.is_none() {
            p.words.last_mut().map_or(&mut p.lead, |w| &mut w.text).push_str(s);
        }
    };
    loop {
        match reader.read_event() {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let a = e.attributes_raw();
                if para.is_none() {
                    if e.local_name().as_ref() == "p" {
                        let (begin, end) = (attr(a, "begin").and_then(ttml_time), attr(a, "end").and_then(ttml_time));
                        para = Some(Para { begin, end, lead: String::new(), words: Vec::new(), depth: 0, skip_from: None });
                    }
                    continue;
                }
                let p = para.as_mut().unwrap();
                p.depth += 1;
                if p.skip_from.is_some() {
                    continue;
                }
                if a.contains("x-translation") || a.contains("x-roman") {
                    p.skip_from = Some(p.depth);
                } else if e.local_name().as_ref() == "span" {
                    if let Some(start_ms) = attr(a, "begin").and_then(ttml_time) {
                        let end_ms = attr(a, "end").and_then(ttml_time).unwrap_or(NO_END);
                        p.words.push(Word { start_ms, end_ms, text: String::new() });
                    }
                }
            }
            Ok(Event::End(_)) => {
                let Some(p) = para.as_mut() else { continue };
                if p.depth > 0 {
                    if p.skip_from == Some(p.depth) {
                        p.skip_from = None;
                    }
                    p.depth -= 1;
                    continue;
                }
                let p = para.take().unwrap();
                let Some(begin) = p.begin else { continue };
                let (text, words) = if p.words.is_empty() {
                    (collapse(&p.lead), Vec::new())
                } else {
                    let mut words = p.words;
                    words[0].text.insert_str(0, &p.lead);
                    normalize(words)
                };
                out.push(Line { start_ms: begin, end_ms: p.end.filter(|&e| e > begin), text, words });
            }
            Ok(Event::Text(t)) => {
                if let Some(p) = para.as_mut() {
                    push(p, &t.xml10_content());
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if let Some(p) = para.as_mut() {
                    let c = match r.resolve_char_ref() {
                        Ok(Some(c)) => c,
                        _ => match &*r {
                            "amp" => '&',
                            "lt" => '<',
                            "gt" => '>',
                            "quot" => '"',
                            "apos" => '\'',
                            _ => continue,
                        },
                    };
                    push(p, c.encode_utf8(&mut [0; 4]));
                }
            }
            _ => {}
        }
    }
    finalize(out)
}

/// TTML clock values: "s.mmm", "m:s.mmm", "h:m:s.mmm", "12.3s".
pub fn ttml_time(v: &str) -> Option<u64> {
    let v = v.trim();
    let v = v.strip_suffix('s').unwrap_or(v);
    let mut s = 0.0f64;
    for part in v.split(':') {
        s = s * 60.0 + part.parse::<f64>().ok()?;
    }
    (s >= 0.0 && s.is_finite()).then(|| (s * 1000.0).round() as u64)
}

/// Matches `<open>\d+:\d+([.:]\d+)?<close>` at the start of `s`; returns (ms, byte length).
fn stamp_at(s: &str, open: u8, close: u8) -> Option<(u64, usize)> {
    let b = s.as_bytes();
    if b.first() != Some(&open) {
        return None;
    }
    let digits = |from: usize| from + b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let m_end = digits(1);
    if m_end == 1 || b.get(m_end) != Some(&b':') {
        return None;
    }
    let s_end = digits(m_end + 1);
    if s_end == m_end + 1 {
        return None;
    }
    let mut end = s_end;
    if matches!(b.get(end), Some(b'.' | b':')) && digits(end + 1) > end + 1 {
        end = digits(end + 1);
    }
    if b.get(end) != Some(&close) {
        return None;
    }
    let min: u64 = s[1..m_end].parse().ok()?;
    let sec: f64 = s[m_end + 1..end].replacen(':', ".", 1).parse().ok()?;
    Some((min.checked_mul(60_000)?.checked_add((sec * 1000.0).round() as u64)?, end + 1))
}

/// `"name=\"value\""` lookup in a raw attribute string, name preceded by a non-word char (JS `\bname="`).
fn attr<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(off) = raw[from..].find(name) {
        let i = from + off;
        from = i + name.len();
        if raw[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some(rest) = raw[from..].strip_prefix('=') else { continue };
        let Some(q) = rest.chars().next().filter(|&c| c == '"' || c == '\'') else { continue };
        let v = &rest[1..];
        return v.find(q).map(|e| &v[..e]);
    }
    None
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapses whitespace across words like the line text; whitespace stays at the end of the word it follows.
fn normalize(raw: Vec<Word>) -> (String, Vec<Word>) {
    let mut words: Vec<Word> = Vec::with_capacity(raw.len());
    let (mut started, mut space) = (false, false);
    for w in raw {
        let mut text = String::with_capacity(w.text.len());
        for c in w.text.chars() {
            if !c.is_whitespace() {
                text.push(c);
                started = true;
                space = false;
            } else if started && !space {
                text.push(' ');
                space = true;
            }
        }
        if text.trim().is_empty() {
            if let Some(prev) = words.last_mut() {
                prev.text.push_str(&text);
            }
            continue;
        }
        words.push(Word { text, ..w });
    }
    if let Some(last) = words.last_mut() {
        if last.text.ends_with(' ') {
            last.text.pop();
        }
    }
    (words.iter().map(|w| w.text.as_str()).collect(), words)
}

fn is_credit(text: &str) -> bool {
    let Some(colon) = text.find([':', '：']) else { return false };
    let prefix = text[..colon].to_lowercase();
    let n = prefix.chars().count();
    CREDIT_KEYS.iter().any(|k| {
        prefix.match_indices(k).any(|(i, _)| {
            let before = prefix[..i].chars().count();
            before <= 30 && n - before - k.chars().count() <= 30
        })
    })
}

/// Shared post-processing: credit/note-only/header lines, sort, empty lines as previous line ends, word ends.
fn finalize(mut out: Vec<Line>) -> Option<Vec<Line>> {
    out.retain(|l| !is_credit(&l.text));
    for l in &mut out {
        if l.text.chars().all(|c| matches!(c, '♪' | '♫' | '♬' | '♩') || c.is_whitespace()) {
            l.text.clear();
            l.words.clear();
        }
    }
    out.sort_by_key(|l| l.start_ms);
    // Sources often open with a "Title - Artist" header line.
    if let Some(i) = out.iter().position(|l| !l.text.is_empty()) {
        if out[i].text.contains(" - ") {
            out.remove(i);
        }
    }
    let mut lines: Vec<Line> = Vec::with_capacity(out.len());
    let mut prev_text = false;
    for l in out {
        if !l.text.is_empty() {
            lines.push(l);
            prev_text = true;
            continue;
        }
        if let Some(p) = lines.last_mut().filter(|p| prev_text && p.end_ms.is_none() && l.start_ms > p.start_ms) {
            p.end_ms = Some(l.start_ms);
        }
        prev_text = false;
    }
    for i in 0..lines.len() {
        let next = lines.get(i + 1).map(|l| l.start_ms);
        let line_end = lines[i].end_ms;
        let words = &mut lines[i].words;
        for j in 0..words.len() {
            if words[j].end_ms == NO_END {
                let end = words.get(j + 1).map(|w| w.start_ms).or(line_end).or(next).unwrap_or(words[j].start_ms);
                words[j].end_ms = end.max(words[j].start_ms);
            }
        }
    }
    (!lines.is_empty()).then_some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(start_ms: u64, end_ms: u64, text: &str) -> Word {
        Word { start_ms, end_ms, text: text.into() }
    }

    #[test]
    fn ttml_words_translation_and_ends() {
        let ttml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><body><div>
<p begin="1.5" end="00:03.250" ttm:agent="v1"><span begin="1.5" end="2.0">Hello</span> <span begin="2.0" end="3.25">wor&amp;ld</span><span ttm:role="x-translation" xml:lang="ko">안녕</span></p>
<p begin="0:0:4.000" end="5s"><span begin="4" end="4.5">Next</span>
  <span ttm:role="x-bg"><span begin="4.5">(ooh)</span></span></p>
<p begin="6">  Plain &lt;line&gt;  <span ttm:role="x-roman">rom</span></p>
</div></body></tt>"#;
        let lines = parse_ttml(ttml).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "Hello wor&ld");
        assert_eq!(lines[0].end_ms, Some(3250));
        assert_eq!(lines[0].words, vec![w(1500, 2000, "Hello "), w(2000, 3250, "wor&ld")]);
        assert_eq!(lines[1].text, "Next (ooh)");
        assert_eq!(lines[1].start_ms, 4000);
        // A span without end runs to the line end.
        assert_eq!(lines[1].words, vec![w(4000, 4500, "Next "), w(4500, 5000, "(ooh)")]);
        assert_eq!((lines[2].text.as_str(), lines[2].end_ms), ("Plain <line>", None));
        assert!(lines[2].words.is_empty());
        for l in &lines {
            assert_eq!(l.words.iter().map(|w| w.text.as_str()).collect::<String>(), if l.words.is_empty() { "" } else { l.text.as_str() });
        }
    }

    #[test]
    fn enhanced_lrc_words() {
        let lines = parse_lrc("[00:01.00]<00:01.00>Hello <00:01.50>world<00:02.25>\n[00:03.00]Plain line").unwrap();
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].end_ms, Some(2250));
        assert_eq!(lines[0].words, vec![w(1000, 1500, "Hello "), w(1500, 2250, "world")]);
        assert_eq!((lines[1].end_ms, lines[1].words.len()), (None, 0));

        // Without a closing tag the last word runs to the next line.
        let lines = parse_lrc("[00:01.00]<00:01.00>a <00:01.50>b\n[00:04.00]c").unwrap();
        assert_eq!(lines[0].words, vec![w(1000, 1500, "a "), w(1500, 4000, "b")]);
        assert_eq!(lines[0].end_ms, None);
    }

    #[test]
    fn multi_stamp_lrc() {
        let lines = parse_lrc("[00:10.00][00:20.00]Chorus\r\n[00:15.00]Verse\r\n[00:16.00]\r\n[ar:Someone]").unwrap();
        let got: Vec<_> = lines.iter().map(|l| (l.start_ms, l.end_ms, l.text.as_str())).collect();
        assert_eq!(got, vec![(10000, None, "Chorus"), (15000, Some(16000), "Verse"), (20000, None, "Chorus")]);

        // Repeated stamps carry their word timings along.
        let lines = parse_lrc("[00:10.00][00:20.00]<00:10.00>la <00:10.50>la<00:11.00>").unwrap();
        assert_eq!(lines[1].words, vec![w(20000, 20500, "la "), w(20500, 21000, "la")]);
        assert_eq!(lines[1].end_ms, Some(21000));
    }

    #[test]
    fn filters_credits_header_and_notes() {
        let lrc = "[00:00.00]Song - Artist\n[00:01.00]作词：某人\n[00:02.00]Lyrics by: Someone\n\
                   [00:05.00]Real line - with dash\n[00:06.00]♪ ♪\n[00:07.00]Time: now";
        let lines = parse_lrc(lrc).unwrap();
        let got: Vec<_> = lines.iter().map(|l| (l.start_ms, l.end_ms, l.text.as_str())).collect();
        assert_eq!(got, vec![(5000, Some(6000), "Real line - with dash"), (7000, None, "Time: now")]);
        assert!(parse_lrc("[ti:Song]\nplain text only").is_none());
    }

    #[test]
    fn time_formats() {
        assert_eq!(ttml_time("12.3s"), Some(12300));
        assert_eq!(ttml_time("1:02.5"), Some(62500));
        assert_eq!(ttml_time("01:00:00.000"), Some(3_600_000));
        assert_eq!(ttml_time("abc"), None);
        assert_eq!(stamp_at("[01:02:50]x", b'[', b']'), Some((62500, 10)));
        assert_eq!(stamp_at("[1:02.345]", b'[', b']'), Some((62345, 10)));
        assert_eq!(stamp_at("[01:02.]", b'[', b']'), None);
        assert_eq!(stamp_at("[ti:x]", b'[', b']'), None);
    }
}
