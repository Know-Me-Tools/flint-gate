//! SSE (Server-Sent Events) parsing and streaming.

use crate::error::{FlintClientError, Result};
use crate::types::SseEvent;
use bytes::{Buf, Bytes};
use futures::{Stream, StreamExt};
use std::collections::VecDeque;
use std::pin::Pin;

/// Internal accumulator for a single in-flight SSE event.
#[derive(Default, Debug)]
struct SseAccumulator {
    event: Option<String>,
    data: Vec<String>,
    id: Option<String>,
}

impl SseAccumulator {
    fn reset(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() && self.event.is_none() && self.id.is_none() {
            return None;
        }
        let joined = self.data.join("\n");
        // An empty `data:` block should still emit a blank event per spec,
        // but we skip purely-comment-only frames with no data and no id.
        if joined.is_empty() && self.id.is_none() && self.event.is_none() {
            self.data.clear();
            return None;
        }
        let ev = SseEvent {
            event: self.event.take().unwrap_or_else(|| "message".to_string()),
            // [DONE] sentinel — emit as data, the consumer can check is_done().
            data: joined,
            id: self.id.take(),
        };
        self.data.clear();
        Some(ev)
    }

    fn handle_line(&mut self, line: &str) {
        // Lines per the SSE spec:
        //   - empty line = event dispatch boundary
        //   - line starting with ':' = comment, ignore
        //   - "field: value" or "field:value" or just "field"
        if line.is_empty() {
            return; // dispatch handled by caller
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.find(':') {
            Some(idx) => {
                let field = &line[..idx];
                // Per spec, strip a single leading space after the colon.
                let mut value = &line[idx + 1..];
                if value.starts_with(' ') {
                    value = &value[1..];
                }
                (field, value)
            }
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            "id" => self.id = Some(value.to_string()),
            "retry" => {
                // retry field is advisory; we ignore it.
            }
            _ => {
                // Unknown field — ignore per spec.
            }
        }
    }
}

/// Parse a complete buffer into a list of zero or more [`SseEvent`]s.
///
/// This is a stateful line-oriented parser. It expects UTF-8 input. Lines may
/// be terminated by `\n`, `\r\n`, or `\r`. Any trailing partial line is
/// retained in `pending` for the next call (carried across chunk boundaries).
///
/// Returns the events emitted in this pass; the unconsumed tail is written
/// back into `pending` so the caller can prepend it to the next chunk.
pub(crate) fn parse_chunk(
    chunk: &mut Bytes,
    pending: &mut String,
) -> Vec<SseEvent> {
    // Append decoded text to the pending buffer.
    pending.push_str(&String::from_utf8_lossy(chunk.as_ref()));
    chunk.advance(chunk.remaining());

    let mut events = Vec::new();
    let mut acc = SseAccumulator::default();

    while let Some(term_idx) = next_newline(pending) {
        let line: String = pending.drain(..term_idx.line_end).collect();
        // Drop the terminator itself.
        pending.drain(..term_idx.term_len);

        if line.is_empty() {
            if let Some(ev) = acc.reset() {
                events.push(ev);
            }
            continue;
        }
        acc.handle_line(&line);
    }

    events
}

#[derive(Clone, Copy)]
struct Term {
    /// Index in the string where the line (excluding terminator) ends.
    line_end: usize,
    /// Length of the terminator (1 or 2).
    term_len: usize,
}

fn next_newline(s: &str) -> Option<Term> {
    let bytes = s.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        let b = bytes[idx];
        if b == b'\n' {
            return Some(Term {
                line_end: idx,
                term_len: 1,
            });
        }
        if b == b'\r' {
            // \r\n or lone \r
            let term_len = if idx + 1 < bytes.len() && bytes[idx + 1] == b'\n' {
                2
            } else {
                1
            };
            return Some(Term {
                line_end: idx,
                term_len,
            });
        }
        idx += 1;
    }
    None
}

/// Convert a `reqwest::Response` byte-stream into a stream of parsed
/// [`SseEvent`]s.
///
/// The returned stream terminates when the underlying body ends, or yields an
/// error if a chunk fails to read or decode.
pub(crate) fn sse_event_stream(
    response: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<SseEvent>> + Send>> {
    use futures::stream;

    struct State<S> {
        bytes: S,
        pending: String,
        queue: VecDeque<SseEvent>,
    }

    let byte_stream = response.bytes_stream();
    let initial = State {
        bytes: byte_stream,
        pending: String::new(),
        queue: VecDeque::new(),
    };

    let stream = stream::unfold(initial, |mut state| async move {
        loop {
            if let Some(ev) = state.queue.pop_front() {
                return Some((Ok(ev), state));
            }
            match state.bytes.next().await {
                Some(Ok(mut chunk)) => {
                    let parsed = parse_chunk(&mut chunk, &mut state.pending);
                    state.queue.extend(parsed);
                    continue;
                }
                Some(Err(e)) => {
                    return Some((Err(FlintClientError::stream(e)), state));
                }
                None => {
                    // Stream ended — flush any trailing partial frame.
                    if !state.pending.is_empty() {
                        state.pending.push('\n');
                        let mut dummy = Bytes::new();
                        let parsed = parse_chunk(&mut dummy, &mut state.pending);
                        state.queue.extend(parsed);
                        state.pending.clear();
                        if let Some(ev) = state.queue.pop_front() {
                            return Some((Ok(ev), state));
                        }
                    }
                    return None;
                }
            }
        }
    });

    Box::pin(stream)
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    use bytes::Bytes;

    fn parse_all(input: &str) -> Vec<SseEvent> {
        let mut pending = String::new();
        let mut chunk = Bytes::copy_from_slice(input.as_bytes());
        parse_chunk(&mut chunk, &mut pending)
    }

    #[test]
    fn parses_single_data_line() {
        let events = parse_all("data: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
        assert_eq!(events[0].event, "message");
        assert_eq!(events[0].id, None);
    }

    #[test]
    fn joins_multi_line_data() {
        let events = parse_all("data: line1\ndata: line2\ndata:line3\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2\nline3");
    }

    #[test]
    fn captures_event_and_id_fields() {
        let events = parse_all("event: ping\nid: 42\ndata: payload\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "ping");
        assert_eq!(events[0].id.as_deref(), Some("42"));
        assert_eq!(events[0].data, "payload");
    }

    #[test]
    fn handles_done_sentinel() {
        let events = parse_all("data: [DONE]\n\n");
        assert_eq!(events.len(), 1);
        assert!(events[0].is_done());
    }

    #[test]
    fn ignores_comments_and_unknown_fields() {
        let events = parse_all(": this is a comment\ndata: ok\nunknown: ignored\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "ok");
    }

    #[test]
    fn dispatches_multiple_events_in_one_chunk() {
        let events = parse_all("data: a\n\ndata: b\n\ndata: c\n\n");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].data, "a");
        assert_eq!(events[2].data, "c");
    }

    #[test]
    fn handles_crlf_line_endings() {
        let events = parse_all("data: crlf\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "crlf");
    }

    #[test]
    fn handles_lone_cr() {
        let events = parse_all("data: cr\r\r");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "cr");
    }

    #[test]
    fn retains_partial_line_across_chunks() {
        let mut pending = String::new();
        let mut first = Bytes::copy_from_slice(b"data: par");
        let ev1 = parse_chunk(&mut first, &mut pending);
        assert!(ev1.is_empty());
        assert_eq!(pending, "data: par");

        let mut second = Bytes::copy_from_slice(b"ted\n\n");
        let ev2 = parse_chunk(&mut second, &mut pending);
        assert_eq!(ev2.len(), 1);
        assert_eq!(ev2[0].data, "parted");
    }

    #[test]
    fn empty_data_field_emits_blank_event_when_id_present() {
        let events = parse_all("id: 7\n\n");
        // id without data is still a valid frame per spec — emits empty data.
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("7"));
        assert_eq!(events[0].data, "");
    }

    #[test]
    fn data_field_with_trailing_space_after_colon_only_drops_one_space() {
        // "data:  two spaces" → leading single space stripped → " two spaces"
        let events = parse_all("data:  two spaces\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, " two spaces");
    }
}
