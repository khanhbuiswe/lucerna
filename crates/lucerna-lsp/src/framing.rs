//! JSON-RPC base protocol framing: `Content-Length` headers plus a body.

use std::io::{self, BufRead, Read, Write};

/// Writes `body` as one framed message and flushes.
///
/// # Errors
/// When the writer fails.
pub fn write_message(w: &mut impl Write, body: &str) -> io::Result<()> {
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(body.as_bytes())?;
    w.flush()
}

/// Reads one framed message. `Ok(None)` at a clean EOF before any header.
///
/// # Errors
/// `InvalidData` for a missing or malformed `Content-Length` or a non-UTF-8
/// body; `UnexpectedEof` when the stream ends mid-message; other read errors.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut length: Option<usize> = None;
    let mut line = String::new();
    let mut first = true;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return if first {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        first = false;
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(
                value
                    .trim()
                    .parse()
                    .map_err(|_| invalid("bad Content-Length"))?,
            );
        }
    }
    let length = length.ok_or_else(|| invalid("missing Content-Length"))?;
    let mut body = Vec::new();
    r.take(length as u64).read_to_end(&mut body)?;
    if body.len() != length {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    String::from_utf8(body)
        .map(Some)
        .map_err(|_| invalid("body is not UTF-8"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(body: &str) -> Vec<u8> {
        let mut out = Vec::new();
        write_message(&mut out, body).unwrap();
        out
    }

    #[test]
    fn reads_two_messages_in_one_buffer() {
        let mut buf = framed("{\"a\":1}");
        buf.extend(framed("héllo"));
        let mut r = buf.as_slice();
        assert_eq!(read_message(&mut r).unwrap().as_deref(), Some("{\"a\":1}"));
        assert_eq!(read_message(&mut r).unwrap().as_deref(), Some("héllo"));
        assert_eq!(read_message(&mut r).unwrap(), None);
    }

    #[test]
    fn accepts_content_type_header_any_order() {
        let raw = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: 2\r\n\r\n{}";
        let mut r = &raw[..];
        assert_eq!(read_message(&mut r).unwrap().as_deref(), Some("{}"));
        let raw = b"Content-Length: 2\r\nContent-Type: x\r\n\r\n{}";
        let mut r = &raw[..];
        assert_eq!(read_message(&mut r).unwrap().as_deref(), Some("{}"));
    }

    #[test]
    fn clean_eof_is_none() {
        let mut r: &[u8] = b"";
        assert_eq!(read_message(&mut r).unwrap(), None);
    }

    #[test]
    fn missing_length_is_invalid_data() {
        let mut r: &[u8] = b"Content-Type: x\r\n\r\n{}";
        let err = read_message(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let mut r: &[u8] = b"Content-Length: abc\r\n\r\n{}";
        assert_eq!(
            read_message(&mut r).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn body_of_64k_bytes() {
        let body = "x".repeat(64 * 1024);
        let buf = framed(&body);
        let mut r = buf.as_slice();
        assert_eq!(read_message(&mut r).unwrap().unwrap().len(), 64 * 1024);
    }

    #[test]
    fn truncated_body_is_unexpected_eof() {
        let mut r: &[u8] = b"Content-Length: 10\r\n\r\nabc";
        assert_eq!(
            read_message(&mut r).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
