//! LSP base protocol: `Content-Length`-framed JSON-RPC messages.

use std::io::{self, BufRead, Write};

use serde_json::Value;

pub fn write_message(w: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

/// Reads one message; `Ok(None)` means the peer closed the connection.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                let parsed = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                length = Some(parsed);
            }
        }
    }
    let mut body = vec![0; length.unwrap_or_default()];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map(Some).map_err(io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_back_to_back_messages() {
        let mut buf = Vec::new();
        write_message(&mut buf, &json!({"id": 1, "result": "é"})).unwrap();
        write_message(&mut buf, &json!({"method": "x"})).unwrap();

        let mut r = io::Cursor::new(buf);
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({"id": 1, "result": "é"})));
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({"method": "x"})));
        assert_eq!(read_message(&mut r).unwrap(), None);
    }

    #[test]
    fn reads_godot_style_unseparated_messages() {
        // Godot sends the next header right after the previous body.
        let raw = "Content-Length: 2\r\n\r\n{}Content-Length: 7\r\n\r\n{\"a\":1}";
        let mut r = io::Cursor::new(raw.as_bytes());
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({})));
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({"a": 1})));
    }
}
