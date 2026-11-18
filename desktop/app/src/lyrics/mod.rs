//! Synced lyrics: local .lrc files, then a disk cache, then online sources (port of lyrics.js).

mod parse;
mod text;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Line, Lyrics, Track};
use parse::{parse_lrc, parse_ttml};
use text::{clean_artist, clean_title, enc, first_artist, title_matches};

const BETTER_API: &str = "https://api.betterlyrics.org";
const LRC_RED: &str = "https://lrc.red";
const KUGOU: &str = "https://lyrics.kugou.com";
const LRCLIB: &str = "https://lrclib.net";
const NETEASE: &str = "https://music.163.com";
const QQ: &str = "https://c.y.qq.com";
const BETTER_RETRY: Duration = Duration::from_secs(8);
const NEGATIVE_TTL_MS: u64 = 3600 * 1000;

type Found = Option<(Vec<Line>, &'static str)>;

struct Query<'a> {
    title: &'a str,
    artist: &'a str,
    first: &'a str,
    album: &'a str,
    dur: u64,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    lyrics: Option<Lyrics>,
    fetched_at_unix_ms: u64,
}

/// Blocking lookup; None when nothing synced is found or `cancel` gets set.
pub fn fetch(track: &Track, lrc_dirs: &[PathBuf], cancel: &AtomicBool) -> Option<Lyrics> {
    let title = clean_title(&track.title);
    let artist = clean_artist(&track.artist);
    if let Some(lines) = local(track, &title, &artist, lrc_dirs) {
        return Some(Lyrics { lines, source: "Local file".into() });
    }
    let dur = (track.length_ms + 500) / 1000;
    let cancelled = || cancel.load(Ordering::Relaxed);
    if dur == 0 || cancelled() {
        return None;
    }

    // Album is part of the key: Better Lyrics matches on it, so a late-arriving album is a different query.
    let cache = dirs::cache_dir().map(|d| {
        let key = format!("{artist}\n{title}\n{dur}\n{}", track.album);
        d.join("lyrics-island").join("desktop").join(format!("{:016x}.json", fnv1a(key.as_bytes())))
    });
    if let Some(hit) = cache.as_deref().and_then(read_cache) {
        return hit;
    }

    let q = Query { title: &title, artist: &artist, first: first_artist(&artist), album: &track.album, dur };
    let mut found = None;
    let providers: [fn(&Query) -> Found; 6] = [lrc_red, better, lrclib, kugou, netease, qq];
    for provider in providers {
        if cancelled() {
            return None;
        }
        found = provider(&q);
        if found.is_some() {
            break;
        }
    }
    // The Better Lyrics browser extension fills the API cache when the song starts; give it time, then retry once.
    if found.is_none() {
        for _ in 0..BETTER_RETRY.as_millis() / 100 {
            if cancelled() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        found = better(&q);
    }
    if cancelled() {
        return None;
    }

    let lyrics = found.map(|(lines, source)| Lyrics { lines, source: source.into() });
    if let Some(path) = cache {
        let entry = CacheEntry { lyrics, fetched_at_unix_ms: now_ms() };
        if let (Some(dir), Ok(json)) = (path.parent(), serde_json::to_vec(&entry)) {
            let _ = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, json));
        }
        return entry.lyrics;
    }
    lyrics
}

/// Blocking GET through the shared agent; the body only for status 200.
pub fn http_get(url: &str, headers: &[(&str, &str)]) -> Option<Vec<u8>> {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder()
            .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
            .user_agent("lyrics-island-desktop/0.1")
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into()
    });
    let mut req = AGENT.get(url);
    for &(k, v) in headers {
        req = req.header(k, v);
    }
    let mut resp = req.call().ok()?;
    if resp.status().as_u16() != 200 {
        return None;
    }
    resp.body_mut().read_to_vec().ok()
}

fn json(url: &str, headers: &[(&str, &str)]) -> Option<Value> {
    serde_json::from_slice(&http_get(url, headers)?).ok()
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Some(lyrics) for a usable entry: positive ones forever, misses for an hour. Corrupt files count as absent.
fn read_cache(path: &Path) -> Option<Option<Lyrics>> {
    let e: CacheEntry = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (e.lyrics.is_some() || now_ms().saturating_sub(e.fetched_at_unix_ms) < NEGATIVE_TTL_MS).then_some(e.lyrics)
}

/// Sibling .lrc of the playing file, then "<artist> - <title>.lrc" / "<title>.lrc" in the configured dirs.
fn local(track: &Track, title: &str, artist: &str, dirs: &[PathBuf]) -> Option<Vec<Line>> {
    let read = |p: &Path| {
        let bytes = std::fs::read(p).ok()?;
        parse_lrc(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}'))
    };
    if let Some(lines) = track.file_url.as_deref().and_then(file_url_path).and_then(|p| read(&p.with_extension("lrc"))) {
        return Some(lines);
    }
    if title.is_empty() {
        return None;
    }
    let mut names = vec![format!("{title}.lrc").to_lowercase()];
    if !artist.is_empty() {
        names.insert(0, format!("{artist} - {title}.lrc").to_lowercase());
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        for name in &names {
            let hit = files.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == *name));
            if let Some(lines) = hit.and_then(|p| read(p)) {
                return Some(lines);
            }
        }
    }
    None
}

fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let (b, mut bytes, mut i) = (rest.as_bytes(), Vec::with_capacity(rest.len()), 0);
    while i < b.len() {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex.filter(|_| b[i] == b'%') {
            Some(v) => {
                bytes.push(v);
                i += 3;
            }
            None => {
                bytes.push(b[i]);
                i += 1;
            }
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // file:///C:/x -> C:/x; file://server/share -> //server/share (UNC).
    #[cfg(windows)]
    let path = match path.strip_prefix('/') {
        Some(p) if p.as_bytes().get(1) == Some(&b':') => p.to_string(),
        Some(_) => path,
        None => format!("//{path}"),
    };
    Some(PathBuf::from(path))
}

fn str_of(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// JSON value as a query parameter (ids come as strings or numbers).
fn param(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}

fn names(v: &Value) -> String {
    v.as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>().join(" ")
}

fn b64_text(s: &str) -> Option<String> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    BASE64_STANDARD.decode(clean).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Entries within ±3 s that pass `ok`, closest length first.
fn ranked(list: &Value, dur: u64, secs: impl Fn(&Value) -> f64, ok: impl Fn(&Value) -> bool) -> Vec<&Value> {
    let diff = |x: &Value| (secs(x) - dur as f64).abs();
    let mut v: Vec<&Value> = list.as_array().into_iter().flatten().filter(|x| diff(x) <= 3.0 && ok(x)).collect();
    v.sort_by(|a, b| diff(a).total_cmp(&diff(b)));
    v
}

/// Candidates matching title, first artist and length.
fn picks<'a>(
    q: &Query,
    list: &'a Value,
    name: impl Fn(&Value) -> &str,
    singers: impl Fn(&Value) -> String,
    secs: impl Fn(&Value) -> f64,
) -> Vec<&'a Value> {
    ranked(list, q.dur, secs, |x| title_matches(name(x), q.title) && title_matches(&singers(x), q.first))
}

fn lrc_red(q: &Query) -> Found {
    let res = json(&format!("{LRC_RED}/api/v1?track={}&artist={}", enc(q.title), enc(q.first)), &[])?;
    let prefix = format!("{LRC_RED}/");
    let hits = ranked(&res["results"], q.dur, |x| x["duration"].as_f64().unwrap_or(0.0), |x| {
        title_matches(str_of(&x["track_name"]), q.title) && str_of(&x["lyricsUrl"]).starts_with(&prefix)
    });
    let ttml = http_get(str_of(&hits.first()?["lyricsUrl"]), &[])?;
    Some((parse_ttml(&String::from_utf8_lossy(&ttml))?, "lrc.red"))
}

/// Better Lyrics serves cached songs keyless; a miss is 401 -> None.
fn better(q: &Query) -> Found {
    let url = format!(
        "{BETTER_API}/getLyrics?s={}&a={}&al={}&d={}",
        enc(q.title),
        enc(q.artist),
        enc(q.album),
        q.dur
    );
    Some((parse_ttml(json(&url, &[])?["ttml"].as_str()?)?, "Better Lyrics"))
}

fn lrclib(q: &Query) -> Found {
    let res = json(&format!("{LRCLIB}/api/search?track_name={}&artist_name={}", enc(q.title), enc(q.first)), &[])?;
    let synced: Value = res.as_array()?.iter().filter(|x| !str_of(&x["syncedLyrics"]).is_empty()).cloned().collect();
    let hits = picks(
        q,
        &synced,
        |x| str_of(&x["trackName"]),
        |x| str_of(&x["artistName"]).to_string(),
        |x| x["duration"].as_f64().unwrap_or(f64::NAN),
    );
    Some((parse_lrc(str_of(&hits.first()?["syncedLyrics"]))?, "LRCLIB"))
}

fn kugou(q: &Query) -> Found {
    let keyword = format!("{} - {}", q.first, q.title);
    let res = json(
        &format!("{KUGOU}/search?ver=1&man=yes&client=pc&keyword={}&duration={}", enc(&keyword), q.dur * 1000),
        &[],
    )?;
    let hits = ranked(&res["candidates"], q.dur, |x| x["duration"].as_f64().unwrap_or(0.0) / 1000.0, |x| {
        title_matches(str_of(&x["song"]), q.title)
    });
    let hit = hits.first()?;
    let url = format!(
        "{KUGOU}/download?ver=1&client=pc&fmt=lrc&charset=utf8&id={}&accesskey={}",
        enc(&param(&hit["id"])),
        enc(&param(&hit["accesskey"]))
    );
    Some((parse_lrc(&b64_text(json(&url, &[])?["content"].as_str()?)?)?, "KuGou"))
}

fn netease(q: &Query) -> Found {
    let s = format!("{} {}", q.first, q.title);
    let res = json(&format!("{NETEASE}/api/search/get?s={}&type=1&limit=10", enc(&s)), &[])?;
    let hits = picks(
        q,
        &res["result"]["songs"],
        |x| str_of(&x["name"]),
        |x| names(&x["artists"]),
        |x| x["duration"].as_f64().unwrap_or(f64::NAN) / 1000.0,
    );
    // Copyright-restricted entries come back with empty lyrics; try the next match.
    hits.iter().take(3).find_map(|hit| {
        let lrc = json(&format!("{NETEASE}/api/song/lyric?id={}&lv=1", param(&hit["id"])), &[])?;
        Some((parse_lrc(lrc["lrc"]["lyric"].as_str()?)?, "Netease Cloud Music"))
    })
}

fn qq(q: &Query) -> Found {
    let headers = [("Referer", "https://y.qq.com/")];
    let w = format!("{} {}", q.first, q.title);
    let res = json(&format!("{QQ}/soso/fcgi-bin/client_search_cp?w={}&format=json&n=10&p=1&cr=1", enc(&w)), &headers)?;
    let hits = picks(
        q,
        &res["data"]["song"]["list"],
        |x| str_of(&x["songname"]),
        |x| names(&x["singer"]),
        |x| x["interval"].as_f64().unwrap_or(f64::NAN),
    );
    let url = format!(
        "{QQ}/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={}&format=json&nobase64=0",
        param(&hits.first()?["songmid"])
    );
    Some((parse_lrc(&b64_text(json(&url, &headers)?["lyric"].as_str()?)?)?, "QQ Music"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_urls() {
        #[cfg(not(windows))]
        assert_eq!(file_url_path("file:///music/A%20B/%EC%95%84.flac"), Some(PathBuf::from("/music/A B/아.flac")));
        #[cfg(windows)]
        assert_eq!(file_url_path("file:///C:/Music/A%20B.flac"), Some(PathBuf::from("C:/Music/A B.flac")));
        assert_eq!(file_url_path("https://x/y"), None);
    }

    #[test]
    fn negative_cache_expires() {
        let dir = std::env::temp_dir().join(format!("lyrics-island-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.json");
        let write = |at: u64| {
            std::fs::write(&path, serde_json::to_vec(&CacheEntry { lyrics: None, fetched_at_unix_ms: at }).unwrap())
                .unwrap()
        };
        write(now_ms());
        assert_eq!(read_cache(&path), Some(None));
        write(now_ms() - NEGATIVE_TTL_MS - 1);
        assert_eq!(read_cache(&path), None);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(read_cache(&path), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
