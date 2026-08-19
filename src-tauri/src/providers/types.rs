// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The vocabulary every provider speaks.
//!
//! These are the types that cross the IPC boundary, so their field names are
//! camelCase. The wire shapes the *providers* demand are a separate matter and
//! live in [`super::wire`] — an OpenAI-compatible endpoint wants `max_tokens`,
//! and mixing the two conventions in one struct is how a request quietly stops
//! being honoured.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// OpenAI's documented ceiling. Values outside it are rejected locally rather
/// than spending a round trip to learn the same thing.
const TEMPERATURE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=2.0;

/// Who said something.
///
/// Serialised lowercase because that is what the OpenAI chat format requires;
/// single lowercase words are identical under camelCase, so this satisfies
/// both conventions at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// The instructions, from `src-tauri/src/prompts`.
    System,
    /// The person using Skia, or the far end of the call.
    User,
    /// A previous model turn, replayed as context.
    Assistant,
}

impl ChatRole {
    /// The wire spelling, for messages and logs alike.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

impl fmt::Display for ChatRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One turn of a conversation.
///
/// Both field names survive the camelCase rename unchanged, which is the only
/// reason this type can be serialised straight into an OpenAI request body.
/// Adding a multi-word field (`toolCallId`, say) breaks that, and the wire
/// module would then need a message struct of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

/// One generation request.
///
/// `model` is allowed to be empty, which means "whatever the resolved provider
/// is configured with". Callers that route by [`ProviderRole`] normally do not
/// know or care which model a role maps to, and forcing them to name one would
/// defeat the point of the registry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub model: String,
    /// `None` leaves the limit to the provider.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// `None` leaves the sampling temperature to the provider.
    #[serde(default)]
    pub temperature: Option<f32>,
}

impl ChatRequest {
    /// A request with a single user turn and the provider's own model.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::user(content)],
            ..Self::default()
        }
    }

    /// Reject what the provider would only reject later, and say why.
    ///
    /// Every provider calls this, the mock included, so an invalid request
    /// fails the same way offline as it does against a real endpoint. The
    /// temperature check is not cosmetic: `serde_json` renders a non-finite
    /// float as `null`, so a NaN that slipped through would be sent as
    /// `"temperature": null` and silently ignored.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.messages.is_empty() {
            return Err(ProviderError::InvalidRequest {
                detail: "there are no messages to send".to_string(),
            });
        }

        if let Some(temperature) = self.temperature {
            if !temperature.is_finite() {
                return Err(ProviderError::InvalidRequest {
                    detail: format!("the temperature {temperature} is not a finite number"),
                });
            }
            if !TEMPERATURE_RANGE.contains(&temperature) {
                return Err(ProviderError::InvalidRequest {
                    detail: format!(
                        "the temperature {temperature} is outside the accepted range {}..={}",
                        TEMPERATURE_RANGE.start(),
                        TEMPERATURE_RANGE.end()
                    ),
                });
            }
        }

        if self.max_tokens == Some(0) {
            return Err(ProviderError::InvalidRequest {
                detail: "maxTokens is 0, so the answer would be empty by construction".to_string(),
            });
        }

        Ok(())
    }
}

/// One streamed fragment of an answer.
///
/// Deltas are emitted as they arrive and are never buffered into a whole
/// message on this side: the point of streaming is the first visible token, and
/// the latency budget in `docs/ARCHITECTURE.md` puts that at 0.3–0.9 s.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delta {
    pub content: String,
}

impl Delta {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

/// Which job the app needs a model for, rather than which model to use.
///
/// Call sites ask for a *role*; the registry decides which configured provider
/// serves it. That indirection is what lets one user run `chat_fast` on Groq
/// and `reason_strict` on a local Ollama model while another runs both on
/// OpenRouter, without a single call site knowing either name.
///
/// The wire spelling is the snake_case alias, because these are identifiers a
/// user writes into configuration rather than object fields — the camelCase
/// convention covers the latter. The camelCase spelling is accepted as an alias
/// so a frontend that normalises everything cannot break routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderRole {
    /// Live-meeting answers. Chosen for time to first token, not for depth.
    #[serde(rename = "chat_fast", alias = "chatFast")]
    ChatFast,
    /// Ask mode and the post-call pack, where being right beats being quick.
    #[serde(rename = "reason_strict", alias = "reasonStrict")]
    ReasonStrict,
    /// Region capture and screenshots, which need an image-capable model.
    #[serde(rename = "vision")]
    Vision,
}

impl ProviderRole {
    /// Every role, so a settings screen can enumerate them without a list of
    /// its own that drifts out of date.
    pub const ALL: [Self; 3] = [Self::ChatFast, Self::ReasonStrict, Self::Vision];

    /// The alias as written in configuration.
    pub fn alias(self) -> &'static str {
        match self {
            Self::ChatFast => "chat_fast",
            Self::ReasonStrict => "reason_strict",
            Self::Vision => "vision",
        }
    }
}

impl fmt::Display for ProviderRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.alias())
    }
}

impl FromStr for ProviderRole {
    type Err = ProviderError;

    /// Accepts both the snake_case and the camelCase spelling, matching what
    /// `serde` accepts.
    fn from_str(alias: &str) -> Result<Self, Self::Err> {
        match alias {
            "chat_fast" | "chatFast" => Ok(Self::ChatFast),
            "reason_strict" | "reasonStrict" => Ok(Self::ReasonStrict),
            "vision" => Ok(Self::Vision),
            _ => Err(ProviderError::UnknownRole {
                alias: alias.to_string(),
            }),
        }
    }
}

/// An API key that cannot be printed by accident.
///
/// The privacy commitment in `docs/ARCHITECTURE.md` says keys live in the OS
/// keychain and never reach a config file or a log. A plain `String` makes that
/// a matter of everyone remembering; this makes it a property of the type —
/// there is no `Display`, no `Deref`, and `Debug` redacts, so the only way to
/// see the key is to ask for it by its full name.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Hand out the key itself. Called once, when building the `Authorization`
    /// header, and the resulting header value is marked sensitive.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Everything that can go wrong reaching a model.
///
/// Each variant names which provider failed, because a user may have three
/// configured and "request failed" tells them nothing about which one to go
/// and fix. Transport failures deliberately drop the URL reqwest attaches to
/// its errors: a user-supplied base URL can carry a key in its query string,
/// and the provider id identifies the failure better anyway.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("the request to {provider} could not be sent: {source}")]
    Transport {
        provider: String,
        source: reqwest::Error,
    },

    #[error("{provider} returned HTTP {status}: {message}")]
    Http {
        provider: String,
        status: u16,
        message: String,
    },

    #[error("{provider} reported an error mid-stream: {message}")]
    Upstream { provider: String, message: String },

    #[error(
        "a streamed chunk from {provider} was not the JSON the OpenAI chat \
         format defines ({source}); the chunk was: {snippet}"
    )]
    Json {
        provider: String,
        snippet: String,
        source: serde_json::Error,
    },

    #[error("the request for {provider} could not be encoded as JSON: {source}")]
    Encode {
        provider: String,
        source: serde_json::Error,
    },

    #[error("the response from {provider} was not a usable event stream: {detail}")]
    Protocol { provider: String, detail: String },

    #[error("{provider} is not configured correctly: {detail}")]
    Config { provider: String, detail: String },

    #[error("this request cannot be sent as it stands: {detail}")]
    InvalidRequest { detail: String },

    #[error("no model provider is configured for the '{role}' role")]
    RoleNotConfigured { role: ProviderRole },

    #[error(
        "the '{role}' role falls back through {tried:?}, but none of those \
         providers is registered"
    )]
    RoleUnavailable {
        role: ProviderRole,
        tried: Vec<String>,
    },

    #[error("a provider with the id '{id}' is already registered")]
    DuplicateProvider { id: String },

    #[error("'{alias}' is not a model role; the roles are chat_fast, reason_strict and vision")]
    UnknownRole { alias: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_message_serialises_into_the_openai_shape() {
        let json = serde_json::to_value(ChatMessage::user("hello")).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "hello");

        assert_eq!(
            serde_json::to_value(ChatMessage::system("be brief")).unwrap()["role"],
            "system"
        );
        assert_eq!(
            serde_json::to_value(ChatMessage::assistant("ok")).unwrap()["role"],
            "assistant"
        );
    }

    #[test]
    fn a_chat_request_uses_camel_case_over_ipc() {
        let request = ChatRequest {
            messages: vec![ChatMessage::user("hi")],
            model: "gpt-4o-mini".to_string(),
            max_tokens: Some(64),
            temperature: Some(0.2),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["maxTokens"], 64);
        assert!(
            json.get("max_tokens").is_none(),
            "the IPC shape is camelCase; snake_case belongs to the wire module"
        );

        // Compared with a tolerance, not for equality: `temperature` is an f32
        // and JSON numbers are f64, so 0.2 widens to 0.20000000298023224. Every
        // provider parses that as the same sampling temperature, and narrowing
        // the field to f64 to make the digits prettier would be the wrong fix.
        let temperature = json["temperature"]
            .as_f64()
            .expect("temperature must serialise as a number");
        assert!((temperature - 0.2).abs() < 1e-6, "got {temperature}");

        let round_tripped: ChatRequest = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, request);
    }

    #[test]
    fn a_delta_is_camel_case_and_round_trips() {
        let json = serde_json::to_value(Delta::new("tok")).unwrap();
        assert_eq!(json["content"], "tok");
        assert_eq!(
            serde_json::from_value::<Delta>(json).unwrap(),
            Delta::new("tok")
        );
    }

    #[test]
    fn validation_rejects_what_a_provider_would_reject_later() {
        assert!(ChatRequest::user("hi").validate().is_ok());

        let empty = ChatRequest::default();
        let error = empty
            .validate()
            .expect_err("a request with no messages must not be sent");
        assert!(error.to_string().contains("no messages"), "{error}");

        for bad in [f32::NAN, f32::INFINITY] {
            let request = ChatRequest {
                temperature: Some(bad),
                ..ChatRequest::user("hi")
            };
            let error = request
                .validate()
                .expect_err("a non-finite temperature would serialise as null");
            assert!(error.to_string().contains("finite"), "{error}");
        }

        let request = ChatRequest {
            temperature: Some(9.0),
            ..ChatRequest::user("hi")
        };
        assert!(request
            .validate()
            .expect_err("9.0 is outside 0..=2")
            .to_string()
            .contains("outside"));

        let request = ChatRequest {
            max_tokens: Some(0),
            ..ChatRequest::user("hi")
        };
        assert!(request
            .validate()
            .expect_err("a zero token budget cannot produce an answer")
            .to_string()
            .contains("maxTokens"));
    }

    #[test]
    fn roles_use_their_configuration_alias_on_the_wire() {
        assert_eq!(
            serde_json::to_value(ProviderRole::ChatFast).unwrap(),
            serde_json::json!("chat_fast")
        );
        assert_eq!(
            serde_json::to_value(ProviderRole::ReasonStrict).unwrap(),
            serde_json::json!("reason_strict")
        );
        assert_eq!(ProviderRole::Vision.to_string(), "vision");

        for role in ProviderRole::ALL {
            assert_eq!(role.alias().parse::<ProviderRole>().unwrap(), role);
            assert_eq!(
                serde_json::from_str::<ProviderRole>(&format!("\"{}\"", role.alias())).unwrap(),
                role
            );
        }

        assert_eq!(
            "chatFast".parse::<ProviderRole>().unwrap(),
            ProviderRole::ChatFast,
            "a camelCased frontend must not break routing"
        );
        assert_eq!(
            serde_json::from_str::<ProviderRole>("\"reasonStrict\"").unwrap(),
            ProviderRole::ReasonStrict
        );

        let error = "chat-fast"
            .parse::<ProviderRole>()
            .expect_err("an unknown alias must not resolve to a default role");
        assert!(error.to_string().contains("chat_fast"), "{error}");
    }

    #[test]
    fn an_api_key_never_prints_itself() {
        let key = ApiKey::new("sk-live-do-not-print-me");

        assert_eq!(format!("{key:?}"), "ApiKey(<redacted>)");
        assert!(!format!("{key:#?}").contains("sk-live"));
        assert!(!format!("{:?}", Some(key.clone())).contains("sk-live"));
        assert_eq!(key.expose_secret(), "sk-live-do-not-print-me");
    }
}
