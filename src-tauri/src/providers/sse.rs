// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Incremental decoding of an OpenAI-compatible SSE response.
//!
//! This is the one place in the provider layer that touches raw bytes, and the
//! only place that knows how a chat completion is framed. Both the real HTTP
//! client and the mock provider hand their bytes to [`delta_stream`], so the
//! offline path and the network path parse identically — a mock that had its own
//! shortcut would prove nothing about the code that ships.
//!
//! ## What makes this fiddly
//!
//! A `Stream` of network chunks has no relationship to the event framing. One
//! chunk can hold six events, or half of a `data:` line, or a single byte in the
//! middle of a UTF-8 sequence. So bytes are accumulated and only ever
//! interpreted a whole line at a time: a partial line stays in the buffer until
//! its newline arrives, which also means a multi-byte character split across two
//! reads is reassembled before anyone tries to decode it.
//!
//! ## What is ignored, and what is not
//!
//! Blank lines separate events, lines beginning with `:` are comments — which is
//! how gateways keep a connection warm — and `event:`, `id:` and `retry:` are
//! fields this decoder has no use for. All of those are skipped, because the SSE
//! specification says to skip what you do not understand.
//!
//! A `data:` payload that is not valid JSON is *not* skipped. A dropped chunk is
//! a hole in the middle of an answer, and an answer with a hole in it that
//! claims to be complete is worse than a visible failure.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::{Stream, StreamExt};

use super::cancel::CancellationToken;
use super::types::{Delta, ProviderError};
use super::wire::{self, CompletionChunk};

/// The field prefix carrying a completion chunk.
const DATA_PREFIX: &[u8] = b"data:";

/// The payload that ends a stream.
const DONE_SENTINEL: &str = "[DONE]";

/// A line this long without a newline is not an event stream, it is a runaway
/// or a hostile endpoint. Base URLs are user-supplied, so the buffer needs a
/// ceiling; a megabyte is far more than any real chunk.
const MAX_PENDING_LINE: usize = 1 << 20;

/// Raw response bytes, however the transport happens to have split them.
pub(super) type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, ProviderError>> + Send>>;

/// The decoded answer, one fragment at a time.
pub type DeltaStream = Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>;

/// Byte-level SSE framing plus chunk parsing, with no opinion about transports.
///
/// Feed it with [`push`](Self::push) as bytes arrive and finish with
/// [`finish`](Self::finish) when the transport closes.
pub(super) struct SseDecoder {
    /// Named in every error this decoder produces: a user with three providers
    /// configured needs to know which one is misbehaving.
    provider: Arc<str>,
    /// Bytes seen but not yet terminated by a newline.
    buffer: Vec<u8>,
    /// Whether any `data:` line at all has arrived.
    saw_event: bool,
    /// Whether `data: [DONE]` has arrived.
    saw_done: bool,
}

impl SseDecoder {
    pub(super) fn new(provider: Arc<str>) -> Self {
        Self {
            provider,
            buffer: Vec::new(),
            saw_event: false,
            saw_done: false,
        }
    }

    /// Whether the stream carried anything that looked like an event.
    ///
    /// A response with no events at all is how a wrong base URL presents
    /// itself: a login page, a JSON object, a 200 from a proxy. It is not an
    /// empty answer, and it must not be reported as one.
    pub(super) fn saw_event(&self) -> bool {
        self.saw_event
    }

    /// Whether the provider signalled a clean end of stream.
    pub(super) fn saw_done(&self) -> bool {
        self.saw_done
    }

    /// Absorb a chunk of response bytes, appending every complete delta in it to
    /// `deltas`.
    ///
    /// Deltas are appended rather than returned so that a failure part-way
    /// through a chunk does not throw away the tokens that arrived before it. One
    /// network read can easily hold five good events and then a broken sixth; the
    /// five are real, the user has a right to them, and the error still has to be
    /// reported afterwards.
    ///
    /// Appending nothing is normal and frequent: keep-alives, the role-only
    /// opening chunk, and any chunk that completes only part of a line all produce
    /// no delta at all.
    pub(super) fn push(
        &mut self,
        chunk: &[u8],
        deltas: &mut Vec<Delta>,
    ) -> Result<(), ProviderError> {
        // Anything after [DONE] is trailing noise, not content. Providers do
        // send a final newline after it.
        if self.saw_done {
            return Ok(());
        }

        self.buffer.extend_from_slice(chunk);

        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
            line.pop();
            // Tolerate both CRLF and bare LF: which one arrives depends on the
            // gateway in front of the model, not on the model.
            if line.last() == Some(&b'\r') {
                line.pop();
            }

            self.handle_line(&line, deltas)?;

            if self.saw_done {
                self.buffer.clear();
                break;
            }
        }

        if self.buffer.len() > MAX_PENDING_LINE {
            return Err(ProviderError::Protocol {
                provider: self.provider.to_string(),
                detail: format!(
                    "{} bytes arrived without a single line break, which no chat \
                     completion produces",
                    self.buffer.len()
                ),
            });
        }

        Ok(())
    }

    /// Called once the transport closes, to drain a final unterminated line.
    ///
    /// Servers do not always put a newline after their last event. A leftover
    /// line is parsed rather than discarded — and if it is a JSON object cut in
    /// half by a dropped connection, parsing it is exactly what surfaces the
    /// truncation instead of hiding it.
    pub(super) fn finish(&mut self, deltas: &mut Vec<Delta>) -> Result<(), ProviderError> {
        if self.saw_done || self.buffer.is_empty() {
            self.buffer.clear();
            return Ok(());
        }

        let mut line = std::mem::take(&mut self.buffer);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        self.handle_line(&line, deltas)
    }

    /// Interpret one complete line, with its terminator already removed.
    fn handle_line(&mut self, line: &[u8], deltas: &mut Vec<Delta>) -> Result<(), ProviderError> {
        // An event separator.
        if line.is_empty() {
            return Ok(());
        }

        // A comment. Gateways send these as keep-alives, sometimes several a
        // second on a slow first token.
        if line.starts_with(b":") {
            return Ok(());
        }

        // `event:`, `id:`, `retry:`, or anything else a future gateway invents.
        // The specification says to ignore fields you do not handle.
        let Some(payload) = line.strip_prefix(DATA_PREFIX) else {
            return Ok(());
        };

        // Only complete lines reach here, so a UTF-8 error is a genuinely
        // broken response rather than a chunk boundary.
        let payload = std::str::from_utf8(payload).map_err(|source| ProviderError::Protocol {
            provider: self.provider.to_string(),
            detail: format!("a data line was not valid UTF-8: {source}"),
        })?;
        let payload = payload.trim();

        self.saw_event = true;

        if payload == DONE_SENTINEL {
            self.saw_done = true;
            return Ok(());
        }

        // `data:` with nothing after it is a heartbeat in some gateways. It
        // counts as an event, but there is nothing to decode.
        if payload.is_empty() {
            return Ok(());
        }

        let chunk: CompletionChunk =
            serde_json::from_str(payload).map_err(|source| ProviderError::Json {
                provider: self.provider.to_string(),
                snippet: wire::truncate(payload),
                source,
            })?;

        if let Some(error) = chunk.error {
            return Err(ProviderError::Upstream {
                provider: self.provider.to_string(),
                message: error.describe(),
            });
        }

        // Only the first choice. Skia never asks for more than one completion,
        // and concatenating several would interleave two different answers.
        if let Some(choice) = chunk.choices.into_iter().next() {
            if let Some(content) = choice.delta.content {
                // An empty string is not a delta; the opening and closing chunks
                // of several providers carry one.
                if !content.is_empty() {
                    deltas.push(Delta { content });
                }
            }
        }

        Ok(())
    }
}

/// Decode a byte stream into deltas, stopping the instant `cancel` trips.
///
/// The returned stream is lazy in a way that matters: nothing is read, and in
/// the HTTP case nothing is even *sent*, until it is first polled. A generation
/// that is cancelled before anyone polls it therefore never reaches the network.
pub(super) fn delta_stream(
    body: ByteStream,
    provider: Arc<str>,
    cancel: CancellationToken,
) -> DeltaStream {
    let state = DecodeState {
        decoder: SseDecoder::new(Arc::clone(&provider)),
        body,
        pending: VecDeque::new(),
        failure: None,
        provider,
        cancel,
        ended: false,
    };

    // Fused, because `unfold` panics if it is polled after finishing and a
    // consumer polling a stream one more time than it needed to is ordinary. A
    // provider layer that can be made to panic by a well-behaved caller is not
    // usable from a Tauri command.
    Box::pin(
        futures_util::stream::unfold(state, |mut state| async move {
            loop {
                // Everything already decoded goes out before another byte is read,
                // because one chunk routinely carries several deltas.
                if let Some(delta) = state.pending.pop_front() {
                    return Some((Ok(delta), state));
                }

                // The failure comes after the deltas that preceded it, never
                // instead of them.
                if let Some(error) = state.failure.take() {
                    return Some((Err(error), state));
                }

                if state.ended {
                    return None;
                }

                let polled = {
                    // Split borrows: the cancellation signal is read while the body
                    // is polled mutably.
                    let DecodeState { body, cancel, .. } = &mut state;
                    tokio::select! {
                        // Cancellation wins a tie. Barge-in should not deliver one
                        // more token just because a chunk happened to land in the
                        // same wake-up.
                        biased;
                        () = cancel.cancelled() => None,
                        // Dropping this future loses nothing: `next` holds no
                        // buffer of its own, so an abandoned poll cannot eat a
                        // chunk that a later poll would have needed.
                        chunk = body.next() => Some(chunk),
                    }
                };

                let Some(chunk) = polled else {
                    // Ending the stream drops the body, which aborts the request.
                    return None;
                };

                let mut decoded = Vec::new();
                let outcome = match chunk {
                    Some(Ok(bytes)) => state.decoder.push(&bytes, &mut decoded),

                    Some(Err(error)) => Err(error),

                    None => {
                        state.ended = true;
                        let flushed = state.decoder.finish(&mut decoded);
                        flushed.and_then(|()| state.no_events_check())
                    }
                };

                // Whatever decoded before the failure is still the user's answer.
                state.pending.extend(decoded);

                match outcome {
                    Ok(()) => {
                        if state.decoder.saw_done() {
                            state.ended = true;
                        }
                    }
                    Err(error) => {
                        state.ended = true;
                        state.failure = Some(error);
                    }
                }
            }
        })
        .fuse(),
    )
}

/// Everything [`delta_stream`] carries between polls.
struct DecodeState {
    body: ByteStream,
    decoder: SseDecoder,
    /// Deltas decoded but not yet yielded.
    pending: VecDeque<Delta>,
    /// An error waiting to be yielded, once `pending` has drained.
    failure: Option<ProviderError>,
    provider: Arc<str>,
    cancel: CancellationToken,
    /// Set once no further bytes will be read, for any reason.
    ended: bool,
}

impl DecodeState {
    /// Refuse to report a response that was never an event stream as an answer.
    ///
    /// A body with no `data:` line in it is not an empty completion, it is the
    /// wrong endpoint: a login page, a proxy's 200, or a gateway that ignored
    /// `stream: true`. Returning an empty answer for it would be the one kind of
    /// swallowed error a user cannot debug.
    fn no_events_check(&self) -> Result<(), ProviderError> {
        if self.decoder.saw_event() {
            return Ok(());
        }

        Err(ProviderError::Protocol {
            provider: self.provider.to_string(),
            detail: "the response carried no server-sent events; the base URL may not be \
                     an OpenAI-compatible /v1 endpoint, or the endpoint may have ignored \
                     stream=true"
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decoder labelled for test output.
    fn decoder() -> SseDecoder {
        SseDecoder::new(Arc::from("test-provider"))
    }

    /// One well-formed completion chunk, as a `data:` line with its blank
    /// separator.
    fn event(content: &str) -> String {
        format!(
            "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\
             \"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{content}\"}},\
             \"finish_reason\":null}}]}}\n\n"
        )
    }

    /// Push a chunk that is expected to decode cleanly, and return its deltas.
    fn push_ok(decoder: &mut SseDecoder, chunk: &[u8]) -> Vec<Delta> {
        let mut deltas = Vec::new();
        decoder
            .push(chunk, &mut deltas)
            .expect("this chunk was expected to decode");
        deltas
    }

    /// Finish a decoder that is expected to have nothing broken left in it.
    fn finish_ok(decoder: &mut SseDecoder) -> Vec<Delta> {
        let mut deltas = Vec::new();
        decoder
            .finish(&mut deltas)
            .expect("the leftover buffer was expected to be clean");
        deltas
    }

    /// Push a chunk that is expected to fail, keeping whatever decoded first.
    fn push_err(decoder: &mut SseDecoder, chunk: &[u8]) -> (Vec<Delta>, ProviderError) {
        let mut deltas = Vec::new();
        let error = decoder
            .push(chunk, &mut deltas)
            .expect_err("this chunk was expected to fail");
        (deltas, error)
    }

    /// Feed a decoder a slice of chunks and flatten what it produced.
    fn decode_all(chunks: &[&[u8]]) -> Result<Vec<String>, ProviderError> {
        let mut decoder = decoder();
        let mut deltas = Vec::new();
        for chunk in chunks {
            decoder.push(chunk, &mut deltas)?;
        }
        decoder.finish(&mut deltas)?;
        Ok(deltas.into_iter().map(|delta| delta.content).collect())
    }

    #[test]
    fn a_normal_stream_decodes_in_order() {
        let body = format!(
            "{}{}{}data: [DONE]\n\n",
            event("Two"),
            event(" audio"),
            event(" streams")
        );

        let decoded = decode_all(&[body.as_bytes()]).unwrap();
        assert_eq!(decoded, ["Two", " audio", " streams"]);
    }

    #[test]
    fn several_events_in_one_chunk_all_come_out() {
        let mut decoder = decoder();
        let body = format!("{}{}", event("a"), event("b"));

        let deltas = push_ok(&mut decoder, body.as_bytes());
        assert_eq!(deltas.len(), 2, "one read can carry several events");
        assert_eq!(deltas[0].content, "a");
        assert_eq!(deltas[1].content, "b");
    }

    #[test]
    fn a_data_line_split_across_two_chunks_is_reassembled() {
        let body = event("hello");
        let split = body.len() / 2;
        let (head, tail) = body.as_bytes().split_at(split);

        // The first half must produce nothing at all: half a JSON object is not
        // an error, it is an incomplete line.
        let mut decoder = decoder();
        assert!(push_ok(&mut decoder, head).is_empty());
        assert!(!decoder.saw_event(), "half a line is not yet an event");

        let deltas = push_ok(&mut decoder, tail);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].content, "hello");
    }

    #[test]
    fn a_stream_split_at_every_possible_byte_decodes_identically() {
        // The split above tests one boundary. This tests all of them, including
        // inside `data`, inside a JSON string, between \r and \n, and inside the
        // multi-byte character.
        let body = format!("{}{}data: [DONE]\n\n", event("Zürich"), event(" ok"));
        let bytes = body.as_bytes();

        for split in 0..=bytes.len() {
            let (head, tail) = bytes.split_at(split);
            let decoded = decode_all(&[head, tail])
                .unwrap_or_else(|error| panic!("split at {split} failed: {error}"));
            assert_eq!(
                decoded,
                ["Zürich", " ok"],
                "split at byte {split} changed the result"
            );
        }
    }

    #[test]
    fn one_byte_at_a_time_still_decodes() {
        let body = format!("{}data: [DONE]\n\n", event("drip"));
        let mut decoder = decoder();
        let mut deltas = Vec::new();

        for byte in body.as_bytes() {
            deltas.extend(push_ok(&mut decoder, &[*byte]));
        }
        deltas.extend(finish_ok(&mut decoder));

        assert_eq!(deltas, [Delta::new("drip")]);
        assert!(decoder.saw_done());
    }

    #[test]
    fn keep_alives_comments_and_unknown_fields_are_skipped() {
        let body = format!(
            ": ping\n\
             \n\
             :\n\
             event: message\n\
             id: 42\n\
             retry: 1000\n\
             {}\
             : another keep-alive\n\
             \n\
             {}\
             data: [DONE]\n\n",
            event("kept"),
            event(" too")
        );

        assert_eq!(decode_all(&[body.as_bytes()]).unwrap(), ["kept", " too"]);
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"crlf\"}}]}\r\n\r\n\
                    data: [DONE]\r\n\r\n";

        assert_eq!(decode_all(&[body.as_bytes()]).unwrap(), ["crlf"]);
    }

    #[test]
    fn a_missing_space_after_the_colon_is_accepted() {
        // The specification makes the space optional, and Ollama omits it.
        let body = "data:{\"choices\":[{\"delta\":{\"content\":\"tight\"}}]}\n\ndata:[DONE]\n\n";

        assert_eq!(decode_all(&[body.as_bytes()]).unwrap(), ["tight"]);
    }

    #[test]
    fn done_ends_the_stream_and_later_bytes_are_ignored() {
        let mut decoder = decoder();
        let body = format!("{}data: [DONE]\n\n{}", event("before"), event("after"));

        let deltas = push_ok(&mut decoder, body.as_bytes());
        assert_eq!(
            deltas,
            [Delta::new("before")],
            "nothing after [DONE] counts"
        );
        assert!(decoder.saw_done());

        assert!(
            push_ok(&mut decoder, event("later").as_bytes()).is_empty(),
            "a decoder that has seen [DONE] stays finished"
        );
        assert!(finish_ok(&mut decoder).is_empty());
    }

    #[test]
    fn empty_and_content_free_chunks_produce_no_deltas() {
        let mut decoder = decoder();

        // The role-only opening chunk, an explicit null, an empty string, a
        // usage-only chunk and a bare `data:` heartbeat.
        let body = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":null}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\n\
                    data: {\"usage\":{\"total_tokens\":3}}\n\n\
                    data:\n\n";

        assert!(push_ok(&mut decoder, body.as_bytes()).is_empty());
        assert!(
            decoder.saw_event(),
            "those were real events, just empty ones"
        );
        assert!(!decoder.saw_done());
    }

    #[test]
    fn malformed_json_is_an_error_and_not_a_skipped_line() {
        let body = format!("{}data: {{not json at all}}\n\n", event("first"));

        let mut decoder = decoder();
        let (deltas, error) = push_err(&mut decoder, body.as_bytes());

        match error {
            ProviderError::Json {
                ref provider,
                ref snippet,
                ..
            } => {
                assert_eq!(provider, "test-provider");
                assert!(snippet.contains("not json"), "the chunk is quoted back");
            }
            other => panic!("expected a JSON error, got {other}"),
        }
        assert!(error.to_string().contains("OpenAI chat"), "{error}");

        assert_eq!(
            deltas,
            [Delta::new("first")],
            "a failure later in the chunk must not discard what already decoded"
        );
    }

    #[test]
    fn a_truncated_final_chunk_is_an_error_rather_than_a_short_answer() {
        // A connection dropped mid-object: no newline, no [DONE].
        let body = format!("{}data: {{\"choices\":[{{\"delta\":{{\"cont", event("half"));

        let mut decoder = decoder();
        assert_eq!(push_ok(&mut decoder, body.as_bytes()), [Delta::new("half")]);

        let mut leftover = Vec::new();
        let error = decoder
            .finish(&mut leftover)
            .expect_err("a half-written chunk must not pass as the end of the answer");
        assert!(matches!(error, ProviderError::Json { .. }), "{error}");
        assert!(leftover.is_empty());
    }

    #[test]
    fn an_empty_stream_yields_nothing_and_records_no_event() {
        let mut empty = decoder();

        assert!(push_ok(&mut empty, b"").is_empty());
        assert!(finish_ok(&mut empty).is_empty());
        assert!(
            !empty.saw_event(),
            "an empty body is what a wrong endpoint looks like"
        );
        assert!(!empty.saw_done());

        // Blank lines only: still framing, still no events.
        let mut blank = decoder();
        assert!(push_ok(&mut blank, b"\n\n\r\n").is_empty());
        assert!(finish_ok(&mut blank).is_empty());
        assert!(!blank.saw_event());
    }

    #[test]
    fn a_non_sse_body_records_no_event() {
        let mut decoder = decoder();
        assert!(push_ok(&mut decoder, b"<html><body>Sign in</body></html>\n").is_empty());
        assert!(finish_ok(&mut decoder).is_empty());
        assert!(
            !decoder.saw_event(),
            "an HTML login page must not read as an empty answer"
        );
    }

    #[test]
    fn a_mid_stream_error_object_stops_the_stream() {
        let body = format!(
            "{}data: {{\"error\":{{\"message\":\"context length exceeded\",\
             \"type\":\"invalid_request_error\"}}}}\n\n",
            event("partial")
        );

        let mut decoder = decoder();
        let (deltas, error) = push_err(&mut decoder, body.as_bytes());

        match error {
            ProviderError::Upstream {
                ref provider,
                ref message,
            } => {
                assert_eq!(provider, "test-provider");
                assert_eq!(message, "context length exceeded (invalid_request_error)");
            }
            other => panic!("expected an upstream error, got {other}"),
        }
        assert_eq!(
            deltas,
            [Delta::new("partial")],
            "the tokens that arrived before the provider gave up are still real"
        );
    }

    #[test]
    fn only_the_first_choice_is_read() {
        let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"one\"}},\
                    {\"index\":1,\"delta\":{\"content\":\"two\"}}]}\n\n";

        assert_eq!(
            decode_all(&[body.as_bytes()]).unwrap(),
            ["one"],
            "two completions must not be interleaved into one answer"
        );
    }

    #[test]
    fn invalid_utf8_in_a_data_line_is_reported() {
        let mut line = b"data: ".to_vec();
        line.extend_from_slice(&[0xff, 0xff]);
        line.push(b'\n');

        let (_, error) = push_err(&mut decoder(), &line);
        match error {
            ProviderError::Protocol { ref detail, .. } => assert!(detail.contains("UTF-8")),
            other => panic!("expected a protocol error, got {other}"),
        }
    }

    #[test]
    fn a_runaway_line_is_cut_off_rather_than_buffered_forever() {
        let flood = vec![b'x'; MAX_PENDING_LINE + 1];

        let (_, error) = push_err(&mut decoder(), &flood);
        assert!(matches!(error, ProviderError::Protocol { .. }), "{error}");
        assert!(error.to_string().contains("line break"), "{error}");
    }

    /// Wrap a fixed set of chunks as a transport for [`delta_stream`].
    fn body_of(chunks: Vec<Result<Vec<u8>, ProviderError>>) -> ByteStream {
        Box::pin(futures_util::stream::iter(chunks))
    }

    /// Collect a delta stream into text, or the first error it produced.
    async fn drain(mut stream: DeltaStream) -> Result<String, ProviderError> {
        let mut text = String::new();
        while let Some(delta) = stream.next().await {
            text.push_str(&delta?.content);
        }
        Ok(text)
    }

    #[tokio::test]
    async fn the_stream_wrapper_reassembles_across_reads() {
        let body = format!("{}{}data: [DONE]\n\n", event("Hel"), event("lo"));
        let bytes = body.into_bytes();
        let (head, tail) = bytes.split_at(bytes.len() / 3);

        let stream = delta_stream(
            body_of(vec![Ok(head.to_vec()), Ok(tail.to_vec())]),
            Arc::from("test-provider"),
            CancellationToken::new(),
        );

        assert_eq!(drain(stream).await.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn a_transport_error_is_passed_through_and_ends_the_stream() {
        let mut stream = delta_stream(
            body_of(vec![
                Ok(event("before").into_bytes()),
                Err(ProviderError::Protocol {
                    provider: "test-provider".to_string(),
                    detail: "the connection dropped".to_string(),
                }),
                Ok(event("never read").into_bytes()),
            ]),
            Arc::from("test-provider"),
            CancellationToken::new(),
        );

        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new("before"));
        let error = stream
            .next()
            .await
            .expect("the error must be yielded")
            .expect_err("a transport failure is not a delta");
        assert!(error.to_string().contains("connection dropped"));
        assert!(
            stream.next().await.is_none(),
            "the stream must not continue past a failure"
        );
    }

    #[tokio::test]
    async fn deltas_decoded_before_a_failure_are_delivered_first() {
        // Everything in one read, so the good events and the broken one are
        // decoded by a single call. They still have to come out in order.
        let body = format!(
            "{}{}data: {{\"choices\":[oops]}}\n\n",
            event("one"),
            event(" two")
        );

        let mut stream = delta_stream(
            body_of(vec![Ok(body.into_bytes())]),
            Arc::from("test-provider"),
            CancellationToken::new(),
        );

        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new("one"));
        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new(" two"));
        assert!(matches!(
            stream.next().await.expect("the error must follow them"),
            Err(ProviderError::Json { .. })
        ));
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn a_finished_stream_tolerates_being_polled_again() {
        let body = format!("{}data: [DONE]\n\n", event("done"));
        let mut stream = delta_stream(
            body_of(vec![Ok(body.into_bytes())]),
            Arc::from("test-provider"),
            CancellationToken::new(),
        );

        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new("done"));
        for _ in 0..3 {
            assert!(
                stream.next().await.is_none(),
                "polling a finished stream must return None, not panic"
            );
        }
    }

    #[tokio::test]
    async fn an_empty_response_is_an_error_not_an_empty_answer() {
        let stream = delta_stream(
            body_of(Vec::new()),
            Arc::from("test-provider"),
            CancellationToken::new(),
        );

        let error = drain(stream)
            .await
            .expect_err("a body with no events must not read as a successful empty answer");
        assert!(
            error.to_string().contains("no server-sent events"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn cancelling_before_the_first_poll_reads_nothing() {
        let cancel = CancellationToken::new();
        cancel.cancel();

        let mut stream = delta_stream(
            body_of(vec![Ok(event("unwanted").into_bytes())]),
            Arc::from("test-provider"),
            cancel,
        );

        assert!(
            stream.next().await.is_none(),
            "an already-cancelled generation must not touch the transport"
        );
    }
}
