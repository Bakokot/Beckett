//! Share codes: a theme as one line of text to paste in a chat.
//! `GDTHEME1:` followed by the deflate-compressed TOML in URL-safe base64.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::Theme;

const PREFIX: &str = "GDTHEME1:";
/// Refuse absurd inputs before decompressing.
const MAX_DECODED: usize = 64 * 1024;

pub fn to_share_code(theme: &Theme) -> String {
    let toml = theme.to_toml().expect("themes always serialize");
    let compressed = miniz_oxide::deflate::compress_to_vec(toml.as_bytes(), 9);
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(compressed))
}

pub fn from_share_code(code: &str) -> Result<Theme, String> {
    // Tolerate surrounding text, and line breaks chat apps insert when
    // wrapping; a space ends the code.
    let start = code.find(PREFIX).ok_or("Not a theme code (it should start with GDTHEME1:)")?;
    let body: String = code[start + PREFIX.len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '\n' | '\r'))
        .filter(|c| !matches!(c, '\n' | '\r'))
        .collect();
    let compressed = URL_SAFE_NO_PAD.decode(body).map_err(|_| "The theme code is damaged")?;
    let toml = miniz_oxide::inflate::decompress_to_vec_with_limit(&compressed, MAX_DECODED)
        .map_err(|_| "The theme code is damaged")?;
    let toml = String::from_utf8(toml).map_err(|_| "The theme code is damaged")?;
    Theme::from_toml(&toml).map_err(|e| format!("The theme code is invalid: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_stays_short() {
        let theme = crate::builtin().remove(0);
        let code = to_share_code(&theme);
        assert!(code.starts_with(PREFIX));
        assert!(code.len() < 1200, "code is {} chars", code.len());
        assert_eq!(from_share_code(&code).unwrap(), theme);
        // Pasted with chat noise and a line break in the middle.
        let (a, b) = code.split_at(40);
        assert_eq!(from_share_code(&format!("here: {a}\n{b} enjoy")).unwrap(), theme);
    }

    #[test]
    fn rejects_garbage() {
        assert!(from_share_code("hello").is_err());
        assert!(from_share_code("GDTHEME1:!!!").is_err());
        assert!(from_share_code("GDTHEME1:aGVsbG8").is_err());
    }
}
