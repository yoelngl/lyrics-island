use std::path::PathBuf;

use crate::lyrics;
use crate::model::{ArtRef, Rgba};

const MAX_SIDE: u32 = 256;

/// Blocking: downloads/reads and decodes album art, downscaled to fit 256 px.
pub fn load(art: &ArtRef) -> Option<Rgba> {
    let owned;
    let bytes: &[u8] = match art {
        ArtRef::Bytes(b) => b.as_slice(),
        ArtRef::Url(u) if u.starts_with("http://") || u.starts_with("https://") => {
            owned = lyrics::http_get(u, &[])?;
            owned.as_slice()
        }
        ArtRef::Url(u) => {
            owned = std::fs::read(file_path(u)?).ok()?;
            owned.as_slice()
        }
    };
    let mut img = image::load_from_memory(bytes)
        .map_err(|e| eprintln!("lyrics-island: can't decode album art: {e}"))
        .ok()?;
    if img.width() > MAX_SIDE || img.height() > MAX_SIDE {
        img = img.thumbnail(MAX_SIDE, MAX_SIDE);
    }
    let img = img.into_rgba8();
    Some(Rgba { width: img.width(), height: img.height(), data: img.into_raw() })
}

/// `file:///a%20b.png` → `/a b.png` (`file:///C:/x` → `C:/x` on Windows).
fn file_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let bytes = percent_decode(&rest[rest.find('/')?..]);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(windows)]
    {
        let s = String::from_utf8(bytes).ok()?;
        Some(PathBuf::from(s.strip_prefix('/').unwrap_or(&s)))
    }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_urls() {
        assert_eq!(percent_decode("/a%20b%e2%82%ac%zz%4"), "/a b€%zz%4".as_bytes());
        #[cfg(unix)]
        assert_eq!(file_path("file:///home/u/Cover%20Art.png"), Some(PathBuf::from("/home/u/Cover Art.png")));
        #[cfg(windows)]
        assert_eq!(file_path("file:///C:/Music/a%20b.jpg"), Some(PathBuf::from("C:/Music/a b.jpg")));
        assert_eq!(file_path("https://x/y.png"), None);
    }

    #[test]
    fn downscales_bytes() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(512, 300, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let img = load(&ArtRef::Bytes(std::sync::Arc::new(png))).unwrap();
        assert_eq!((img.width, img.height), (256, 150));
        assert_eq!(&img.data[..4], &[1, 2, 3, 255]);
    }
}
