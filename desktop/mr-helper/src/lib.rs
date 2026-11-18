//! Loaded into /usr/bin/perl via DynaLoader and run as XSUBs: since macOS 15.4 MediaRemote only answers
//! entitled Apple binaries, and perl is one. Empty on other OSes so the workspace builds everywhere.

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::hash_map::DefaultHasher;
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::fmt::Write as _;
    use std::hash::{Hash, Hasher};
    use std::io::Write as _;
    use std::process::exit;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use block2::{Block, RcBlock};
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSData, NSDataBase64EncodingOptions, NSDate, NSDictionary, NSNumber, NSString};

    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    }

    type GetNowPlayingInfo = unsafe extern "C" fn(queue: *mut c_void, block: &Block<dyn Fn(*mut AnyObject)>);
    type SendCommand = unsafe extern "C" fn(command: u32, options: *const c_void) -> bool;
    type SetElapsedTime = unsafe extern "C" fn(elapsed: f64);

    const RTLD_LAZY: c_int = 1;

    fn symbol(name: &CStr) -> Option<*mut c_void> {
        let handle = unsafe { dlopen(c"/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote".as_ptr(), RTLD_LAZY) };
        if handle.is_null() {
            return None;
        }
        let sym = unsafe { dlsym(handle, name.as_ptr()) };
        (!sym.is_null()).then_some(sym)
    }

    #[derive(Default, PartialEq)]
    struct Info {
        title: String,
        artist: String,
        album: String,
        duration: f64,
        elapsed: f64,
        /// Seconds since 1970 at which `elapsed` was sampled by the player.
        timestamp: f64,
        rate: f64,
        /// 0 when there is no artwork.
        art_hash: u64,
    }

    /// Reads the now-playing dictionary; the base64 artwork is only produced when its hash differs from `last_art`.
    unsafe fn read_info(dict: *mut AnyObject, last_art: u64) -> (Info, Option<String>) {
        let Some(dict) = (unsafe { (dict as *const NSDictionary<NSString, AnyObject>).as_ref() }) else {
            return (Info::default(), (last_art != 0).then(String::new));
        };
        let get = |key: &str| dict.objectForKey(&NSString::from_str(key));
        let string = |key: &str| get(key).and_then(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string()).unwrap_or_default();
        let number = |key: &str| get(key).and_then(|o| o.downcast::<NSNumber>().ok()).map_or(0.0, |n| n.doubleValue());
        let art = get("kMRMediaRemoteNowPlayingInfoArtworkData").and_then(|o| o.downcast::<NSData>().ok());
        let art_hash = art.as_ref().map_or(0, |data| {
            let mut h = DefaultHasher::new();
            unsafe { data.as_bytes_unchecked() }.hash(&mut h);
            h.finish() | 1
        });
        let art_b64 = (art_hash != last_art)
            .then(|| art.map_or(String::new(), |d| d.base64EncodedStringWithOptions(NSDataBase64EncodingOptions(0)).to_string()));
        let info = Info {
            title: string("kMRMediaRemoteNowPlayingInfoTitle"),
            artist: string("kMRMediaRemoteNowPlayingInfoArtist"),
            album: string("kMRMediaRemoteNowPlayingInfoAlbum"),
            duration: number("kMRMediaRemoteNowPlayingInfoDuration"),
            elapsed: number("kMRMediaRemoteNowPlayingInfoElapsedTime"),
            timestamp: get("kMRMediaRemoteNowPlayingInfoTimestamp")
                .and_then(|o| o.downcast::<NSDate>().ok())
                .map_or(0.0, |d| d.timeIntervalSince1970()),
            rate: number("kMRMediaRemoteNowPlayingInfoPlaybackRate"),
            art_hash,
        };
        (info, art_b64)
    }

    fn json_str(out: &mut String, s: &str) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }

    fn to_json(info: &Info, art_b64: Option<&str>) -> String {
        if info.title.is_empty() {
            return "{}".into();
        }
        let mut elapsed = info.elapsed;
        if info.timestamp > 0.0 {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
            elapsed += (now - info.timestamp).max(0.0) * info.rate;
        }
        let mut out = String::from("{\"title\":");
        json_str(&mut out, &info.title);
        out.push_str(",\"artist\":");
        json_str(&mut out, &info.artist);
        out.push_str(",\"album\":");
        json_str(&mut out, &info.album);
        let _ = write!(
            out,
            ",\"duration_ms\":{},\"elapsed_ms\":{},\"rate\":{}",
            (info.duration.max(0.0) * 1000.0) as u64,
            (elapsed.max(0.0) * 1000.0) as u64,
            info.rate
        );
        if let Some(b64) = art_b64 {
            out.push_str(",\"artwork_b64\":");
            json_str(&mut out, b64);
        }
        out.push('}');
        out
    }

    /// Prints one JSON line per now-playing change, forever; exits when stdout closes (the app went away).
    #[no_mangle]
    pub extern "C" fn lyrics_island_stream(_interp: *mut c_void, _cv: *mut c_void) {
        let Some(sym) = symbol(c"MRMediaRemoteGetNowPlayingInfo") else { exit(1) };
        let get_info: GetNowPlayingInfo = unsafe { std::mem::transmute(sym) };
        let queue = unsafe { dispatch_get_global_queue(0, 0) };
        let (tx, rx) = mpsc::channel();
        let mut last: Option<Info> = None;
        let mut stdout = std::io::stdout();
        loop {
            let tx = tx.clone();
            let last_art = last.as_ref().map_or(0, |i| i.art_hash);
            let block = RcBlock::new(move |dict: *mut AnyObject| {
                let _ = tx.send(unsafe { read_info(dict, last_art) });
            });
            unsafe { get_info(queue, &block) };
            if let Ok((info, art_b64)) = rx.recv_timeout(Duration::from_secs(2)) {
                if last.as_ref() != Some(&info) {
                    let line = to_json(&info, art_b64.as_deref());
                    if writeln!(stdout, "{line}").and_then(|_| stdout.flush()).is_err() {
                        exit(0);
                    }
                    last = Some(info);
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// Sends the command named by $LYRICS_ISLAND_CMD and exits.
    #[no_mangle]
    pub extern "C" fn lyrics_island_send(_interp: *mut c_void, _cv: *mut c_void) {
        if std::env::var("LYRICS_ISLAND_CMD").as_deref() == Ok("seek") {
            let Some(ms) = std::env::var("LYRICS_ISLAND_POS_MS").ok().and_then(|s| s.parse::<u64>().ok()) else {
                exit(2);
            };
            let Some(sym) = symbol(c"MRMediaRemoteSetElapsedTime") else { exit(1) };
            let set_elapsed: SetElapsedTime = unsafe { std::mem::transmute(sym) };
            unsafe { set_elapsed(ms as f64 / 1000.0) };
            exit(0);
        }
        let command = match std::env::var("LYRICS_ISLAND_CMD").as_deref() {
            Ok("play_pause") => 2, // kMRTogglePlayPause
            Ok("next") => 4,       // kMRNextTrack
            Ok("previous") => 5,   // kMRPreviousTrack
            _ => exit(2),
        };
        let Some(sym) = symbol(c"MRMediaRemoteSendCommand") else { exit(1) };
        let send: SendCommand = unsafe { std::mem::transmute(sym) };
        let ok = unsafe { send(command, std::ptr::null()) };
        exit(if ok { 0 } else { 1 });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn json_escapes_and_omits_art() {
            let info = Info { title: "a\"b\\\n".into(), duration: 1.5, rate: 0.0, ..Default::default() };
            let line = to_json(&info, None);
            assert_eq!(line, r#"{"title":"a\"b\\\n","artist":"","album":"","duration_ms":1500,"elapsed_ms":0,"rate":0}"#);
            assert_eq!(to_json(&Info::default(), Some("x")), "{}");
        }
    }
}
