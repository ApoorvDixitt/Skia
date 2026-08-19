// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! A provider that answers without a network, an account or a key.
//!
//! The roadmap lists a mock provider under the developer panel for a reason:
//! Skia is bring-your-own-key, so the first thing a contributor hits is a
//! feature they cannot run. Everything above the provider layer — streaming
//! Markdown, barge-in, the latency view, the answer card — can be built and
//! tested against this instead.
//!
//! ## Why it goes the long way round
//!
//! A mock that pushed [`Delta`]s straight into a channel would be shorter and
//! would prove nothing. So this one renders its canned answer as real
//! `text/event-stream` bytes and hands them to [`super::sse::delta_stream`],
//! the same decoder the HTTP client uses. Every chunk it emits is a
//! [`CompletionChunk`] serialised by `serde`, so JSON escaping is exercised too.
//!
//! It goes one step further and splits every event across two byte chunks, in
//! the middle of the JSON. Over a real connection a `data:` line straddling two
//! reads is a matter of luck and packet size; here it happens on every token of
//! every run, so the partial-line path can never rot unnoticed.

use std::sync::Arc;
use std::time::Duration;

use super::cancel::CancellationToken;
use super::sse::{self, DeltaStream};
use super::types::{ChatRequest, Delta, ProviderError};
use super::wire::{ChunkChoice, ChunkDelta, CompletionChunk};
use super::Provider;

/// The default canned answer.
///
/// Deliberately contains a double quote, a colon and a newline: all three have
/// to survive JSON encoding to come back out intact, and a newline that escaped
/// unencoded would break SSE framing outright.
pub const DEFAULT_ANSWER: &str = "Two audio streams, never merged.\n\
                                  The microphone and the \"far end\" stay separate all the way \
                                  to transcription, because that separation is what makes \
                                  speaker labelling possible.";

/// The model name reported when none is given, so a latency view has something
/// honest to show.
const DEFAULT_MODEL: &str = "mock-1";

/// A deterministic, offline provider.
///
/// Same answer, same token boundaries, same order, every time — which is what
/// makes it usable as a fixture as well as a development stand-in.
#[derive(Debug, Clone)]
pub struct MockProvider {
    id: Arc<str>,
    model: String,
    tokens: Vec<String>,
    token_delay: Duration,
}

impl MockProvider {
    /// A provider streaming [`DEFAULT_ANSWER`] with no delay between tokens.
    pub fn new(id: impl Into<String>) -> Self {
        Self::with_answer(id, DEFAULT_ANSWER)
    }

    /// A provider streaming `answer`, split into tokens.
    pub fn with_answer(id: impl Into<String>, answer: &str) -> Self {
        Self::with_tokens(id, tokenize(answer))
    }

    /// A provider streaming exactly these tokens, for a test that cares where
    /// the boundaries fall.
    pub fn with_tokens(id: impl Into<String>, tokens: Vec<String>) -> Self {
        Self {
            id: Arc::from(id.into()),
            model: DEFAULT_MODEL.to_string(),
            tokens,
            token_delay: Duration::ZERO,
        }
    }

    /// Wait this long before each token.
    ///
    /// Zero — the default — keeps tests instant. A few tens of milliseconds
    /// imitates a real provider closely enough to build a streaming UI against,
    /// and is what makes barge-in testable: a cancellation landing during one of
    /// these waits is exactly the case that matters.
    pub fn token_delay(mut self, delay: Duration) -> Self {
        self.token_delay = delay;
        self
    }

    /// Report a different model name.
    pub fn model_name(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// The tokens this provider will stream, in order.
    ///
    /// Exposed so a test asserts against the provider's own definition of the
    /// answer rather than a second copy of it that can drift.
    pub fn tokens(&self) -> &[String] {
        &self.tokens
    }

    /// The whole answer, as the caller will have reassembled it.
    pub fn answer(&self) -> String {
        self.tokens.concat()
    }

    /// Render the canned answer as `text/event-stream` bytes.
    ///
    /// Returns the byte chunks to yield, each flagged with whether the per-token
    /// delay belongs before it.
    fn encode(&self) -> Result<Vec<(bool, Vec<u8>)>, ProviderError> {
        // Two chunks per token, plus the opening and closing events.
        let mut chunks = Vec::with_capacity((self.tokens.len() + 2) * 2);

        // Real providers open with a role-only delta carrying no content. It is
        // free to emit and it keeps the "event with nothing in it" path warm.
        chunks.extend(self.encode_event(false, &CompletionChunk::default())?);

        for token in &self.tokens {
            let chunk = CompletionChunk {
                choices: vec![ChunkChoice {
                    delta: ChunkDelta {
                        content: Some(token.clone()),
                    },
                }],
                error: None,
            };
            chunks.extend(self.encode_event(true, &chunk)?);
        }

        let (head, tail) = split_in_two("data: [DONE]\n\n");
        chunks.push((false, head));
        chunks.push((false, tail));

        Ok(chunks)
    }

    /// One chunk as an SSE event, split across two byte chunks.
    fn encode_event(
        &self,
        delay_before: bool,
        chunk: &CompletionChunk,
    ) -> Result<[(bool, Vec<u8>); 2], ProviderError> {
        let json = serde_json::to_string(chunk).map_err(|source| ProviderError::Encode {
            provider: self.id.to_string(),
            source,
        })?;

        let (head, tail) = split_in_two(&format!("data: {json}\n\n"));
        Ok([(delay_before, head), (false, tail)])
    }
}

impl Provider for MockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn stream_chat(&self, request: ChatRequest, cancel: CancellationToken) -> DeltaStream {
        // The mock validates too. An invalid request has to fail the same way
        // offline as it does against a real endpoint, or the offline path stops
        // being a rehearsal for the real one.
        let chunks = match request.validate().and_then(|()| self.encode()) {
            Ok(chunks) => chunks,
            Err(error) => {
                let failure: Result<Delta, ProviderError> = Err(error);
                return Box::pin(futures_util::stream::once(async move { failure }));
            }
        };

        let delay = self.token_delay;
        let body =
            futures_util::stream::unfold(chunks.into_iter(), move |mut remaining| async move {
                let (delay_before, bytes) = remaining.next()?;
                if delay_before && !delay.is_zero() {
                    // Awaited inside the body stream, so the decoder's cancellation
                    // race covers it: a token cancelled mid-wait stops here rather
                    // than after the wait finishes.
                    tokio::time::sleep(delay).await;
                }
                Some((Ok(bytes), remaining))
            });

        sse::delta_stream(Box::pin(body), Arc::clone(&self.id), cancel)
    }
}

/// Split an answer into streamable tokens.
///
/// Whitespace stays attached to the token it follows, so concatenating the
/// tokens reproduces the answer byte for byte. That matters more than imitating
/// any particular tokeniser: a UI that joins deltas must get the original text
/// back.
fn tokenize(answer: &str) -> Vec<String> {
    answer
        .split_inclusive(char::is_whitespace)
        .map(str::to_string)
        .collect()
}

/// Cut a string in half by bytes, which may land inside a character.
///
/// That is the point. A real read boundary has no respect for character
/// boundaries either, and the decoder has to reassemble both.
fn split_in_two(event: &str) -> (Vec<u8>, Vec<u8>) {
    let bytes = event.as_bytes();
    let middle = bytes.len() / 2;
    (bytes[..middle].to_vec(), bytes[middle..].to_vec())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use futures_util::StreamExt;

    use super::*;

    /// Every delta the provider streams, or the first error it produced.
    async fn stream(provider: &MockProvider, cancel: CancellationToken) -> Vec<Delta> {
        let mut stream = provider.stream_chat(ChatRequest::user("go"), cancel);
        let mut deltas = Vec::new();
        while let Some(delta) = stream.next().await {
            deltas.push(delta.expect("the mock must not fail"));
        }
        deltas
    }

    #[tokio::test]
    async fn the_canned_answer_arrives_token_by_token_in_order() {
        let provider = MockProvider::new("mock");
        let deltas = stream(&provider, CancellationToken::new()).await;

        let contents: Vec<&str> = deltas.iter().map(|d| d.content.as_str()).collect();
        let expected: Vec<&str> = provider.tokens().iter().map(String::as_str).collect();
        assert_eq!(
            contents, expected,
            "the deltas must be the provider's own tokens, in order"
        );
        assert!(
            deltas.len() > 5,
            "a canned answer of one delta proves nothing"
        );
    }

    #[tokio::test]
    async fn the_deltas_reassemble_into_the_answer_exactly() {
        let provider = MockProvider::new("mock");
        let joined: String = stream(&provider, CancellationToken::new())
            .await
            .iter()
            .map(|delta| delta.content.as_str())
            .collect();

        assert_eq!(joined, DEFAULT_ANSWER);
        assert_eq!(joined, provider.answer());
        assert!(
            joined.contains('\n') && joined.contains('"'),
            "the round trip has to survive a newline and a quote to mean anything"
        );
    }

    #[tokio::test]
    async fn the_stream_terminates_and_stays_terminated() {
        let provider = MockProvider::with_answer("mock", "one two");
        let mut stream = provider.stream_chat(ChatRequest::user("go"), CancellationToken::new());

        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new("one "));
        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new("two"));
        assert!(stream.next().await.is_none(), "[DONE] must end the stream");
        assert!(
            stream.next().await.is_none(),
            "a finished stream must stay finished when polled again"
        );
    }

    #[tokio::test]
    async fn two_runs_produce_identical_output() {
        let provider = MockProvider::new("mock");
        assert_eq!(
            stream(&provider, CancellationToken::new()).await,
            stream(&provider, CancellationToken::new()).await,
            "a fixture that varies between runs is not a fixture"
        );
    }

    #[tokio::test]
    async fn exact_tokens_are_streamed_verbatim() {
        let provider = MockProvider::with_tokens(
            "mock",
            vec!["Zü".to_string(), "rich".to_string(), " 🎧".to_string()],
        );

        let deltas = stream(&provider, CancellationToken::new()).await;
        assert_eq!(
            deltas,
            [Delta::new("Zü"), Delta::new("rich"), Delta::new(" 🎧")],
            "multi-byte characters must survive being cut in half mid-chunk"
        );
    }

    #[tokio::test]
    async fn an_empty_answer_streams_nothing_but_still_succeeds() {
        let provider = MockProvider::with_tokens("mock", Vec::new());

        assert!(
            stream(&provider, CancellationToken::new()).await.is_empty(),
            "no tokens means no deltas, and the opening event plus [DONE] means no error"
        );
    }

    #[tokio::test]
    async fn the_per_token_delay_is_honoured() {
        let provider =
            MockProvider::with_answer("mock", "a b c").token_delay(Duration::from_millis(30));

        let started = Instant::now();
        let deltas = stream(&provider, CancellationToken::new()).await;
        let elapsed = started.elapsed();

        assert_eq!(deltas.len(), 3);
        assert!(
            elapsed >= Duration::from_millis(60),
            "three tokens at 30 ms each cannot finish in {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn cancelling_stops_the_stream_early() {
        let provider =
            MockProvider::with_answer("mock", "one two three four five six seven eight nine ten")
                .token_delay(Duration::from_millis(20));
        let cancel = CancellationToken::new();
        let mut stream = provider.stream_chat(ChatRequest::user("go"), cancel.clone());

        let first = stream
            .next()
            .await
            .expect("a first delta")
            .expect("no error");
        let second = stream
            .next()
            .await
            .expect("a second delta")
            .expect("no error");
        assert_eq!(first.content, "one ");
        assert_eq!(second.content, "two ");

        cancel.cancel();

        // The remaining eight tokens would take another 160 ms. Cancellation has
        // to end the stream well inside that, not merely stop it eventually.
        let ended = tokio::time::timeout(Duration::from_millis(80), stream.next())
            .await
            .expect("cancellation must not wait for the rest of the answer");
        assert!(
            ended.is_none(),
            "a cancelled stream must end, not yield another delta"
        );
    }

    #[tokio::test]
    async fn cancelling_before_the_first_poll_yields_nothing_at_all() {
        let provider = MockProvider::new("mock");
        let cancel = CancellationToken::new();
        cancel.cancel();

        assert!(
            stream(&provider, cancel).await.is_empty(),
            "an answer abandoned before it started must produce no tokens"
        );
    }

    #[tokio::test]
    async fn an_invalid_request_fails_offline_exactly_as_it_would_online() {
        let provider = MockProvider::new("mock");
        let mut stream = provider.stream_chat(ChatRequest::default(), CancellationToken::new());

        let error = stream
            .next()
            .await
            .expect("the failure must be reported as an item")
            .expect_err("a request with no messages is not answerable");
        assert!(error.to_string().contains("no messages"), "{error}");
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn tokenizing_preserves_the_original_text() {
        for answer in [
            "",
            "one",
            "one two",
            "double  space",
            "trailing space ",
            "line\nbreak",
            "tabs\tand\nnewlines ",
            "Zürich 🎧 ok",
        ] {
            let tokens = tokenize(answer);
            assert_eq!(
                tokens.concat(),
                answer,
                "tokenising {answer:?} lost or added something"
            );
            assert!(
                tokens.iter().all(|token| !token.is_empty()),
                "an empty token would be a delta with no content"
            );
        }

        assert_eq!(tokenize("one two"), ["one ", "two"]);
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn the_identity_and_model_are_reported() {
        let provider = MockProvider::new("offline");
        assert_eq!(provider.id(), "offline");
        assert_eq!(provider.model(), DEFAULT_MODEL);
        assert_eq!(
            MockProvider::new("offline").model_name("fake-70b").model(),
            "fake-70b"
        );
    }

    #[test]
    fn every_event_really_is_split_across_two_chunks() {
        let provider = MockProvider::with_answer("mock", "a b");
        let chunks = provider.encode().unwrap();

        // One opening event, two tokens, one [DONE]: four events, eight chunks.
        assert_eq!(chunks.len(), 8);
        assert_eq!(
            chunks.iter().filter(|(delay, _)| *delay).count(),
            2,
            "the delay belongs once per token, not once per byte chunk"
        );

        for (index, (_, bytes)) in chunks.iter().enumerate() {
            assert!(!bytes.is_empty(), "chunk {index} is empty");
        }

        // No single chunk is a whole event, which is the property that keeps the
        // partial-line path exercised.
        for pair in chunks.chunks(2) {
            let whole: Vec<u8> = pair.iter().flat_map(|(_, bytes)| bytes.clone()).collect();
            assert!(
                whole.ends_with(b"\n\n"),
                "an event must end with a blank line"
            );
            assert!(
                !pair[0].1.ends_with(b"\n\n"),
                "the first half of an event must not be a complete event"
            );
        }
    }
}
