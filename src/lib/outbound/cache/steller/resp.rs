//! RESP codec, client side: encodes commands and parses replies.
//!
//! Commands go out as arrays of bulk strings, the only form steller accepts. Replies
//! are parsed from a buffer that may hold part of a frame, a whole frame, or more.

use thiserror::Error;

/// Longest header line accepted: a sigil plus a length or a short message.
const MAX_LINE: usize = 1024;

/// Largest bulk string accepted. A snapshot is a few hundred bytes; this only stops a
/// corrupt length prefix from asking for gigabytes.
const MAX_BULK: usize = 16 * 1024 * 1024;

/// A reply that does not follow the protocol.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RespError {
    /// The first byte is not a reply type this client reads.
    #[error("unexpected reply type {0:?}")]
    UnknownSigil(char),
    /// A length or integer that is not a number.
    #[error("malformed number in reply")]
    MalformedNumber,
    /// A bulk string whose payload is not followed by `\r\n`.
    #[error("bulk string is missing its terminator")]
    MissingTerminator,
    /// A header line or bulk string past the accepted size.
    #[error("reply exceeds the size limit")]
    TooLarge,
}

/// One reply from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// `+OK\r\n`
    Simple(Vec<u8>),
    /// `-ERR message\r\n`
    Error(Vec<u8>),
    /// `:42\r\n`
    Integer(i64),
    /// `$5\r\nhello\r\n`. Arbitrary bytes, `\r\n` included.
    Bulk(Vec<u8>),
    /// `$-1\r\n`, the reply for a key that does not exist.
    Null,
}

/// Encodes a command as an array of bulk strings.
pub fn encode(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", parts.len()).into_bytes();
    for part in parts {
        out.extend_from_slice(format!("${}\r\n", part.len()).as_bytes());
        out.extend_from_slice(part);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Parses one reply from the front of `buf`.
///
/// Returns the reply and how many bytes it took, or `None` when `buf` does not hold a
/// whole frame yet. The caller reads more and calls again with the longer buffer.
pub fn parse(buf: &[u8]) -> Result<Option<(Reply, usize)>, RespError> {
    let Some((&sigil, after_sigil)) = buf.split_first() else {
        return Ok(None);
    };
    let Some(line_len) = after_sigil.windows(2).position(|pair| pair == b"\r\n") else {
        if after_sigil.len() > MAX_LINE {
            return Err(RespError::TooLarge);
        }
        return Ok(None);
    };
    let (line, after_line) = after_sigil.split_at(line_len);
    // Sigil, line, and the line's `\r\n`.
    let header_len = line_len.saturating_add(3);
    let payload = after_line.get(2..).unwrap_or_default();

    let reply = match sigil {
        b'+' => Reply::Simple(line.to_vec()),
        b'-' => Reply::Error(line.to_vec()),
        b':' => Reply::Integer(number(line)?),
        b'$' => return bulk(number(line)?, payload, header_len),
        other => return Err(RespError::UnknownSigil(char::from(other))),
    };
    Ok(Some((reply, header_len)))
}

fn bulk(len: i64, payload: &[u8], header_len: usize) -> Result<Option<(Reply, usize)>, RespError> {
    if len == -1 {
        return Ok(Some((Reply::Null, header_len)));
    }
    let len = usize::try_from(len).map_err(|_| RespError::MalformedNumber)?;
    if len > MAX_BULK {
        return Err(RespError::TooLarge);
    }
    let Some((value, after_value)) = payload.split_at_checked(len) else {
        return Ok(None);
    };
    match after_value.get(..2) {
        Some(b"\r\n") => Ok(Some((
            Reply::Bulk(value.to_vec()),
            header_len.saturating_add(len).saturating_add(2),
        ))),
        Some(_) => Err(RespError::MissingTerminator),
        None => Ok(None),
    }
}

fn number(line: &[u8]) -> Result<i64, RespError> {
    std::str::from_utf8(line)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or(RespError::MalformedNumber)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole(bytes: &[u8]) -> Reply {
        let (reply, used) = parse(bytes).unwrap().unwrap();
        assert_eq!(used, bytes.len());
        reply
    }

    #[test]
    fn encodes_a_command_as_an_array_of_bulk_strings() {
        assert_eq!(
            encode(&[b"SET", b"k", b"v\r\n"]),
            b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$3\r\nv\r\n\r\n"
        );
    }

    #[test]
    fn parses_each_reply_type() {
        assert_eq!(whole(b"+OK\r\n"), Reply::Simple(b"OK".to_vec()));
        assert_eq!(whole(b"-ERR nope\r\n"), Reply::Error(b"ERR nope".to_vec()));
        assert_eq!(whole(b":-7\r\n"), Reply::Integer(-7));
        assert_eq!(whole(b"$5\r\nhello\r\n"), Reply::Bulk(b"hello".to_vec()));
        assert_eq!(whole(b"$0\r\n\r\n"), Reply::Bulk(Vec::new()));
        assert_eq!(whole(b"$-1\r\n"), Reply::Null);
    }

    #[test]
    fn a_bulk_string_carries_crlf_and_non_utf8_bytes() {
        let payload = b"a\r\nb\x00\xff\xfe";
        let mut frame = format!("${}\r\n", payload.len()).into_bytes();
        frame.extend_from_slice(payload);
        frame.extend_from_slice(b"\r\n");
        assert_eq!(whole(&frame), Reply::Bulk(payload.to_vec()));
    }

    #[test]
    fn every_proper_prefix_of_a_frame_is_incomplete() {
        for frame in [
            &b"+OK\r\n"[..],
            b"-ERR nope\r\n",
            b":12345\r\n",
            b"$-1\r\n",
            b"$10\r\nhello\r\nwor\r\n",
        ] {
            for cut in 0..frame.len() {
                assert_eq!(
                    parse(&frame[..cut]),
                    Ok(None),
                    "cut at {cut} of {:?}",
                    String::from_utf8_lossy(frame)
                );
            }
            assert!(parse(frame).unwrap().is_some());
        }
    }

    #[test]
    fn reports_how_much_it_took_when_more_follows() {
        let (reply, used) = parse(b"+OK\r\n+PONG\r\n").unwrap().unwrap();
        assert_eq!(reply, Reply::Simple(b"OK".to_vec()));
        assert_eq!(used, 5);
    }

    #[test]
    fn rejects_what_is_not_the_protocol() {
        assert_eq!(parse(b"*1\r\n"), Err(RespError::UnknownSigil('*')));
        assert_eq!(parse(b"$abc\r\n"), Err(RespError::MalformedNumber));
        assert_eq!(parse(b"$-2\r\n"), Err(RespError::MalformedNumber));
        assert_eq!(parse(b":1.5\r\n"), Err(RespError::MalformedNumber));
        assert_eq!(parse(b"$2\r\nhiXX"), Err(RespError::MissingTerminator));
    }

    #[test]
    fn rejects_an_oversized_frame() {
        assert_eq!(
            parse(format!("${}\r\n", MAX_BULK + 1).as_bytes()),
            Err(RespError::TooLarge)
        );
        let endless = vec![b'+'; MAX_LINE + 2];
        assert_eq!(parse(&endless), Err(RespError::TooLarge));
    }
}
