//! What a request may be, on both sides of the bridge: one line of at most `MAX_LINE` bytes,
//! and tool arguments that are a JSON object. The peer is an agent CLI on the bridge's stdin,
//! or whoever connects to the app's socket: it is not trusted with the memory of the process,
//! and `AsyncBufReadExt::lines` keeps a line of any length.

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// The longest request line that is read: 1 MiB. A tool call is a few kilobytes.
pub(super) const MAX_LINE: usize = 1 << 20;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Line {
    /// The line without its line end.
    Bytes(Vec<u8>),
    /// The line was longer than the limit: it was read to its end and not kept.
    TooLong,
}

/// The next line of `reader`, of at most `max` bytes. None = the input ended. A longer line is
/// read to its end, so the line after it is the next request; at most `max` bytes of it are
/// ever held.
pub(super) async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R, max: usize) -> std::io::Result<Option<Line>> {
    let mut line: Vec<u8> = vec![];
    let (mut any, mut too_long) = (false, false);
    loop {
        let buf = reader.fill_buf().await?;
        if buf.is_empty() {
            if !any {
                return Ok(None);
            }
            break;
        }
        any = true;
        let end = buf.iter().position(|b| *b == b'\n');
        let part = &buf[..end.unwrap_or(buf.len())];
        if !too_long {
            if line.len() + part.len() > max {
                too_long = true;
                line = vec![];
            } else {
                line.extend_from_slice(part);
            }
        }
        let used = end.map_or(buf.len(), |at| at + 1);
        reader.consume(used);
        if end.is_some() {
            break;
        }
    }
    if too_long {
        return Ok(Some(Line::TooLong));
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some(Line::Bytes(line)))
}

/// What the sender of a line that was too long is told.
pub(super) fn too_long() -> String {
    format!("The request is too long: one call to Codly is at most {} KB. Send less text.", MAX_LINE / 1024)
}

/// The arguments of a tool call. They are a JSON object; a call without arguments (none, or
/// null) has an empty one. Anything else is refused: no tool reads it.
pub(super) fn arguments(given: Option<&Value>) -> Result<Value, String> {
    match given {
        None | Some(Value::Null) => Ok(json!({})),
        Some(args) if args.is_object() => Ok(args.clone()),
        Some(_) => Err("The arguments of a tool call must be a JSON object, like {\"query\": \"…\"}.".into()),
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::*;

    async fn lines(input: &[u8], max: usize, buffer: usize) -> Vec<Line> {
        let mut reader = BufReader::with_capacity(buffer, input);
        let mut found = vec![];
        while let Some(line) = read_line(&mut reader, max).await.unwrap() {
            found.push(line);
        }
        found
    }

    fn text(s: &str) -> Line {
        Line::Bytes(s.as_bytes().to_vec())
    }

    #[tokio::test]
    async fn a_line_past_the_limit_is_dropped_and_the_next_one_is_read() {
        let input = b"one\n\ntwo\r\n0123456789\nthree\nlast";
        // A buffer smaller than the lines, and one that holds the whole input.
        for buffer in [4, 64] {
            let found = lines(input, 8, buffer).await;
            assert_eq!(found, [text("one"), text(""), text("two"), Line::TooLong, text("three"), text("last")], "buffer of {buffer}");
        }
        // Exactly the limit is read; the line end does not count.
        assert_eq!(lines(b"12345678\n123456789\n", 8, 4).await, [text("12345678"), Line::TooLong]);
        // An input that ends inside a line that is too long.
        assert_eq!(lines(b"ok\n123456789", 8, 4).await, [text("ok"), Line::TooLong]);
        assert!(lines(b"", 8, 4).await.is_empty());
    }

    #[test]
    fn only_an_object_is_taken_as_arguments() {
        assert_eq!(arguments(None), Ok(json!({})));
        assert_eq!(arguments(Some(&Value::Null)), Ok(json!({})));
        assert_eq!(arguments(Some(&json!({ "query": "login" }))), Ok(json!({ "query": "login" })));
        for other in [json!("query"), json!(["query"]), json!(7), json!(true)] {
            assert!(arguments(Some(&other)).unwrap_err().contains("must be a JSON object"), "{other}");
        }
    }
}
