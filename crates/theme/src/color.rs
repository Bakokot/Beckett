use std::fmt;

use serde::{Deserialize, Serialize};

/// An sRGB color, written in theme files as `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn from_hex(hex: &str) -> Result<Self, ParseColorError> {
        let err = || ParseColorError(hex.to_string());
        let digits = hex.strip_prefix('#').ok_or_else(err)?;
        if !digits.is_ascii() || !matches!(digits.len(), 6 | 8) {
            return Err(err());
        }
        let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| err());
        Ok(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if digits.len() == 8 { byte(6)? } else { 255 },
        })
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

impl TryFrom<String> for Color {
    type Error = ParseColorError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_hex(&value)
    }
}

impl From<Color> for String {
    fn from(color: Color) -> Self {
        color.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseColorError(String);

impl fmt::Display for ParseColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid color `{}` (expected #rrggbb or #rrggbbaa)", self.0)
    }
}

impl std::error::Error for ParseColorError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats() {
        let c = Color::from_hex("#478cbf").unwrap();
        assert_eq!(c, Color::rgb(0x47, 0x8c, 0xbf));
        assert_eq!(c.to_string(), "#478cbf");

        let c = Color::from_hex("#cdcfd280").unwrap();
        assert_eq!(c.a, 0x80);
        assert_eq!(c.to_string(), "#cdcfd280");
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["478cbf", "#478cb", "#478cbfz0", "#ééé"] {
            assert!(Color::from_hex(bad).is_err(), "{bad}");
        }
    }
}
