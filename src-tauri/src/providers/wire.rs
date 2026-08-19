// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The JSON shapes the OpenAI chat-completions API dictates.
//!
//! These are deliberately separate from the types in [`super::types`]. Those
//! are Skia's, cross IPC and are camelCase; these are somebody else's, cross the
//! network and are snake_case. One struct cannot be both, and the failure mode
//! of pretending otherwise is silent — a `maxTokens` field is not an error to a
//! provider, it is simply not a token limit.
//!
//! Reading is deliberately lenient about *unknown* fields and strict about
//! *malformed* ones. Every gateway adds something: `usage`, `logprobs`,
//! `system_fingerprint`, Groq's `x_groq`, reasoning traces. Rejecting a chunk
//! for carrying one would break a working provider on its next release, so
//! unknown fields are ignored — but a chunk that is not JSON at all is an
//! error, never a skipped line.

use serde::{Deserialize, Serialize};

use super::types::ChatMessage;

/// How much of a provider's error body is worth quoting back to the user.
const MAX_QUOTED_BODY: usize = 512;

/// A streaming chat-completions request.
///
/// The lifetime is there so a request can be encoded without cloning the
/// message list, which on a live-meeting turn carries the whole retrieved
/// context.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct CompletionRequest<'a> {
    pub(super) model: &'a str,
    pub(super) messages: &'a [ChatMessage],
    /// Always `true`. Skia has no non-streaming path: the latency budget in
    /// `docs/ARCHITECTURE.md` is written in time-to-first-token, which a
    /// buffered response cannot report at all.
    pub(super) stream: bool,
    /// Omitted rather than sent as `null`, because not every OpenAI-compatible
    /// gateway treats those the same way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) temperature: Option<f32>,
}

/// One `data:` payload from a streaming completion.
///
/// Serialisable as well as deserialisable, so [`super::mock`] can emit real
/// chunks of this exact shape instead of hand-rolled strings. That is what lets
/// the mock provider and the HTTP client share one decoder: there is only one
/// definition of a chunk, and both sides use it.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct CompletionChunk {
    /// Absent on the chunks some gateways use to carry usage or a heartbeat.
    #[serde(default)]
    pub(super) choices: Vec<ChunkChoice>,
    /// Present when a provider gives up part-way through a 200 response, which
    /// OpenAI, OpenRouter and Groq all do. Without this the stream would just
    /// stop early and look like a short answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<ApiError>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ChunkChoice {
    /// The final chunk of some providers has `finish_reason` and no `delta`.
    #[serde(default)]
    pub(super) delta: ChunkDelta,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ChunkDelta {
    /// `None` covers both "absent" and an explicit `null`, which is what the
    /// role-only opening chunk and the closing chunk both send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<String>,
}

/// The error object every OpenAI-compatible provider wraps its failures in.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ApiError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) message: Option<String>,
    /// e.g. `invalid_request_error`. Kept because the message alone is often
    /// too vague to act on.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub(super) kind: Option<String>,
}

impl ApiError {
    /// The most useful sentence that can be built from what the provider sent.
    pub(super) fn describe(&self) -> String {
        match (self.message.as_deref(), self.kind.as_deref()) {
            (Some(message), Some(kind)) => format!("{message} ({kind})"),
            (Some(message), None) => message.to_string(),
            (None, Some(kind)) => kind.to_string(),
            (None, None) => "the error carried no message".to_string(),
        }
    }
}

/// An error response body, which is not streamed and so is not a chunk.
#[derive(Debug, Deserialize)]
pub(super) struct ErrorEnvelope {
    pub(super) error: ApiError,
}

/// Turn a failed response body into something worth showing a user.
///
/// Providers put the useful sentence inside `error.message`; gateways and
/// reverse proxies return plain text or HTML. Both are handled, and the result
/// is truncated, because a 404 from a mistyped base URL can be an entire HTML
/// page and none of it belongs in an error message.
pub(super) fn describe_error_body(body: &[u8]) -> String {
    if let Ok(envelope) = serde_json::from_slice::<ErrorEnvelope>(body) {
        return truncate(&envelope.error.describe());
    }

    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        "the response had no body".to_string()
    } else {
        truncate(text)
    }
}

/// Shorten to [`MAX_QUOTED_BODY`] characters, counting characters rather than
/// bytes so a multi-byte character is never cut in half.
pub(super) fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_QUOTED_BODY {
        return text.to_string();
    }

    let kept: String = text.chars().take(MAX_QUOTED_BODY).collect();
    format!("{kept}… (truncated)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::types::ChatRole;

    #[test]
    fn a_request_serialises_to_the_snake_case_wire_shape() {
        let messages = vec![ChatMessage::system("be brief"), ChatMessage::user("hi")];
        let request = CompletionRequest {
            model: "llama-3.1-8b-instant",
            messages: &messages,
            stream: true,
            max_tokens: Some(128),
            temperature: Some(0.1),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "llama-3.1-8b-instant");
        assert_eq!(json["stream"], true);
        assert_eq!(json["max_tokens"], 128);
        // An f32 widened to a JSON f64 is 0.10000000149011612, which is the same
        // temperature to every provider that reads it.
        let temperature = json["temperature"].as_f64().expect("a number");
        assert!((temperature - 0.1).abs() < 1e-6, "got {temperature}");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][1]["content"], "hi");
        assert!(
            json.get("maxTokens").is_none(),
            "the provider expects max_tokens; camelCase would be ignored"
        );
    }

    #[test]
    fn absent_options_are_omitted_rather_than_nulled() {
        let messages = vec![ChatMessage::user("hi")];
        let request = CompletionRequest {
            model: "m",
            messages: &messages,
            stream: true,
            max_tokens: None,
            temperature: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("max_tokens").is_none());
        assert!(json.get("temperature").is_none());
    }

    #[test]
    fn a_chunk_survives_unknown_fields_and_a_missing_delta() {
        let chunk: CompletionChunk = serde_json::from_str(
            r#"{"id":"c1","object":"chat.completion.chunk","system_fingerprint":"fp",
                "x_groq":{"queue_time":0.1},
                "choices":[{"index":0,"delta":{"role":"assistant","content":"Hi"},
                            "logprobs":null,"finish_reason":null}]}"#,
        )
        .expect("a real-world chunk with extra fields must still parse");
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hi"));
        assert!(chunk.error.is_none());

        let closing: CompletionChunk =
            serde_json::from_str(r#"{"choices":[{"index":0,"finish_reason":"stop"}]}"#).unwrap();
        assert_eq!(closing.choices[0].delta.content, None);

        let usage_only: CompletionChunk =
            serde_json::from_str(r#"{"usage":{"total_tokens":7}}"#).unwrap();
        assert!(usage_only.choices.is_empty());

        let nulled: CompletionChunk =
            serde_json::from_str(r#"{"choices":[{"delta":{"content":null}}]}"#).unwrap();
        assert_eq!(nulled.choices[0].delta.content, None);
    }

    #[test]
    fn a_mid_stream_error_object_is_recognised() {
        let chunk: CompletionChunk = serde_json::from_str(
            r#"{"error":{"message":"rate limit reached","type":"rate_limit_error"}}"#,
        )
        .unwrap();
        let error = chunk.error.expect("the error object must be picked up");
        assert_eq!(error.describe(), "rate limit reached (rate_limit_error)");

        let bare: ApiError = serde_json::from_str(r#"{"message":"nope"}"#).unwrap();
        assert_eq!(bare.describe(), "nope");
        let typed: ApiError = serde_json::from_str(r#"{"type":"server_error"}"#).unwrap();
        assert_eq!(typed.describe(), "server_error");
        let empty: ApiError = serde_json::from_str("{}").unwrap();
        assert!(empty.describe().contains("no message"));
    }

    #[test]
    fn an_error_body_is_described_whatever_shape_it_arrives_in() {
        assert_eq!(
            describe_error_body(br#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#),
            "Incorrect API key provided (invalid_request_error)"
        );

        assert_eq!(
            describe_error_body(b"  <html><body>404 Not Found</body></html>  "),
            "<html><body>404 Not Found</body></html>",
            "a proxy that answers in HTML must still produce a readable error"
        );

        assert_eq!(describe_error_body(b""), "the response had no body");
        assert_eq!(describe_error_body(b"   "), "the response had no body");

        // Invalid UTF-8 must not panic; it is a body like any other.
        assert!(!describe_error_body(&[0xff, 0xfe]).is_empty());
    }

    #[test]
    fn a_long_body_is_truncated_on_a_character_boundary() {
        let long = "é".repeat(MAX_QUOTED_BODY * 2);
        let short = truncate(&long);

        assert!(short.ends_with("… (truncated)"));
        assert_eq!(
            short.chars().filter(|c| *c == 'é').count(),
            MAX_QUOTED_BODY,
            "truncation counts characters, not bytes"
        );

        assert_eq!(truncate("short"), "short");
    }

    #[test]
    fn a_chunk_round_trips_so_the_mock_can_emit_what_the_decoder_reads() {
        let chunk = CompletionChunk {
            choices: vec![ChunkChoice {
                delta: ChunkDelta {
                    content: Some("a \"quoted\" line\nand another".to_string()),
                },
            }],
            error: None,
        };

        let encoded = serde_json::to_string(&chunk).unwrap();
        assert!(
            !encoded.contains("\"error\""),
            "an absent error must be omitted, not sent as null: {encoded}"
        );
        assert!(
            !encoded.contains('\n'),
            "a newline inside content must be escaped, or it would break SSE framing: {encoded}"
        );

        let decoded: CompletionChunk = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            decoded.choices[0].delta.content.as_deref(),
            Some("a \"quoted\" line\nand another")
        );
    }

    #[test]
    fn the_message_shape_matches_the_wire_shape_without_translation() {
        // CompletionRequest serialises ChatMessage directly, which is only
        // correct because every ChatRole spelling is the wire spelling.
        assert_eq!(ChatRole::System.as_str(), "system");
        assert_eq!(ChatRole::User.as_str(), "user");
        assert_eq!(ChatRole::Assistant.as_str(), "assistant");
    }
}
