//! `file://` URI conversions. Godot percent-encodes the drive colon
//! (`file:///C%3A/...`), so decoding must be tolerant.

use std::fmt::Write;
use std::path::{Path, PathBuf};

pub fn path_to_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s);
    let mut uri = String::from("file://");
    if !s.starts_with('/') {
        uri.push('/');
    }
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                uri.push(b as char)
            }
            _ => write!(uri, "%{b:02X}").unwrap(),
        }
    }
    uri
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let decoded = percent_decode(uri.strip_prefix("file://")?)?;
    Some(decoded_to_path(&decoded, cfg!(windows)))
}

/// A comparison key for paths: separators unified, case-folded on Windows.
pub fn path_key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).trim_end_matches('/');
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

fn decoded_to_path(decoded: &str, windows: bool) -> PathBuf {
    if !windows {
        return PathBuf::from(decoded);
    }
    // "/C:/Users/x" -> "C:\Users\x"
    let b = decoded.as_bytes();
    let trimmed = if b.len() >= 3 && b[0] == b'/' && b[2] == b':' {
        &decoded[1..]
    } else {
        decoded
    };
    PathBuf::from(trimmed.replace('/', "\\"))
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_windows_paths() {
        assert_eq!(
            path_to_uri(Path::new(r"C:\Users\me\My Game\player.gd")),
            "file:///C:/Users/me/My%20Game/player.gd"
        );
        assert_eq!(path_to_uri(Path::new("/home/me/game")), "file:///home/me/game");
    }

    #[test]
    fn decodes_godot_uris() {
        let decoded = percent_decode("/C%3A/Users/me/My%20Game/player.gd").unwrap();
        assert_eq!(
            decoded_to_path(&decoded, true),
            PathBuf::from(r"C:\Users\me\My Game\player.gd")
        );
        assert_eq!(
            decoded_to_path("/home/me/game.gd", false),
            PathBuf::from("/home/me/game.gd")
        );
        assert_eq!(percent_decode("%zz"), None);
    }

    #[test]
    fn path_keys_ignore_separators() {
        assert_eq!(path_key(Path::new("C:/a/b/")), path_key(Path::new("C:/a/b")));
        if cfg!(windows) {
            assert!(same_path(Path::new(r"C:\Users\Me"), Path::new("c:/users/me")));
        }
    }
}
