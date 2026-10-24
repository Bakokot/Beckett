//! Key presses to the byte sequences a shell expects (xterm conventions,
//! which ConPTY also understands).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// Text produced by the key (already layout-mapped).
    Char(String),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1..F12.
    F(u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    /// The xterm modifier parameter (1 = none).
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }

    fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }
}

/// Encodes a key press. `app_cursor` is the terminal's application cursor
/// mode (DECCKM), which changes arrow/Home/End sequences.
pub fn encode_key(key: &Key, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    let csi = |code: &str| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{code}", mods.param()).into_bytes()
        } else if app_cursor {
            format!("\x1bO{code}").into_bytes()
        } else {
            format!("\x1b[{code}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[{code};{}~", mods.param()).into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let bytes = match key {
        Key::Char(text) => return encode_text(text, mods),
        Key::Enter => alt_prefixed(b"\r".to_vec(), mods),
        Key::Backspace => alt_prefixed(if mods.ctrl { vec![0x08] } else { vec![0x7f] }, mods),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Escape => b"\x1b".to_vec(),
        Key::Up => csi("A"),
        Key::Down => csi("B"),
        Key::Right => csi("C"),
        Key::Left => csi("D"),
        Key::Home => csi("H"),
        Key::End => csi("F"),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let code = [b'P', b'Q', b'R', b'S'][usize::from(n - 1)] as char;
            if mods.any() {
                format!("\x1b[1;{}{code}", mods.param()).into_bytes()
            } else {
                format!("\x1bO{code}").into_bytes()
            }
        }
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        Key::F(_) => return None,
    };
    Some(bytes)
}

fn encode_text(text: &str, mods: Mods) -> Option<Vec<u8>> {
    if text.is_empty() {
        return None;
    }
    // Ctrl+Alt is AltGr on many layouts: send the produced text as is.
    if mods.ctrl && mods.alt {
        return Some(text.as_bytes().to_vec());
    }
    if mods.ctrl {
        let mut chars = text.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return Some(text.as_bytes().to_vec());
        };
        let control = match c.to_ascii_lowercase() {
            c @ 'a'..='z' => c as u8 - b'a' + 1,
            '@' | ' ' | '2' => 0,
            '[' | '3' => 0x1b,
            '\\' | '4' => 0x1c,
            ']' | '5' => 0x1d,
            '^' | '6' => 0x1e,
            '_' | '-' | '7' => 0x1f,
            _ => return Some(text.as_bytes().to_vec()),
        };
        return Some(alt_prefixed(vec![control], mods));
    }
    Some(alt_prefixed(text.as_bytes().to_vec(), mods))
}

fn alt_prefixed(mut bytes: Vec<u8>, mods: Mods) -> Vec<u8> {
    if mods.alt && !mods.ctrl {
        bytes.insert(0, 0x1b);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: Key, mods: Mods) -> Vec<u8> {
        encode_key(&key, mods, false).unwrap()
    }

    const CTRL: Mods = Mods { ctrl: true, alt: false, shift: false };
    const ALT: Mods = Mods { ctrl: false, alt: true, shift: false };
    const NONE: Mods = Mods { ctrl: false, alt: false, shift: false };

    #[test]
    fn plain_and_control_characters() {
        assert_eq!(enc(Key::Char("a".into()), NONE), b"a");
        assert_eq!(enc(Key::Char("é".into()), NONE), "é".as_bytes());
        assert_eq!(enc(Key::Char("c".into()), CTRL), [0x03]);
        assert_eq!(enc(Key::Char("C".into()), CTRL), [0x03]);
        assert_eq!(enc(Key::Char("[".into()), CTRL), [0x1b]);
        assert_eq!(enc(Key::Char("b".into()), ALT), [0x1b, b'b']);
        let altgr = Mods { ctrl: true, alt: true, shift: false };
        assert_eq!(enc(Key::Char("{".into()), altgr), b"{");
    }

    #[test]
    fn special_keys() {
        assert_eq!(enc(Key::Enter, NONE), b"\r");
        assert_eq!(enc(Key::Backspace, NONE), [0x7f]);
        assert_eq!(enc(Key::Up, NONE), b"\x1b[A");
        assert_eq!(encode_key(&Key::Up, NONE, true).unwrap(), b"\x1bOA");
        assert_eq!(enc(Key::Right, CTRL), b"\x1b[1;5C");
        assert_eq!(enc(Key::Delete, NONE), b"\x1b[3~");
        assert_eq!(enc(Key::PageUp, Mods { shift: true, ..NONE }), b"\x1b[5;2~");
        assert_eq!(enc(Key::F(1), NONE), b"\x1bOP");
        assert_eq!(enc(Key::F(5), NONE), b"\x1b[15~");
        assert_eq!(enc(Key::F(12), NONE), b"\x1b[24~");
        assert_eq!(enc(Key::Tab, Mods { shift: true, ..NONE }), b"\x1b[Z");
    }
}
