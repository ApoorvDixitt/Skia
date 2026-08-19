// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The model gateway: how Skia reaches a language model.
//!
//! `docs/ARCHITECTURE.md` gives this layer one job and two constraints. The job
//! is streaming chat completion. The constraints are that **nothing leaves the
//! device unless the user configured it** — outbound traffic goes only to the
//! provider the user chose — and that keys live in the OS keychain, never in a
//! config file or a log.
//!
//! ## Shape
//!
//! ```text
//!   caller ──asks for a role──►  Registry
//!                                   │ resolves, in configured order
//!                                   ▼
//!                            dyn Provider
//!                          ┌────────┴────────┐
//!                 OpenAiCompatible      MockProvider
//!                    (real HTTP)      (canned, offline)
//!                          └────────┬────────┘
//!                                   ▼
//!                          one SSE decoder, one
//!                          cancellation race
//! ```
//!
//! Both providers hand raw bytes to the same decoder. The mock is not a
//! shortcut around the real path, it is the real path with a different
//! transport, which is the only way a test against it says anything about the
//! code that ships.
//!
//! ## Why the trait returns a stream instead of being `async`
//!
//! [`Provider`] has no `async fn`, so there is no `async-trait` and no hidden
//! allocation per call: `stream_chat` is an ordinary method returning a boxed
//! [`Stream`](futures_util::Stream). That keeps the trait object-safe with one
//! `Box` per *generation* rather than one per await, and it makes the laziness
//! explicit — a returned stream has sent nothing until it is first polled.
//!
//! ## Cancellation is a requirement, not a nicety
//!
//! Live mode answers speculatively, so a generation has to be abandonable the
//! instant the speaker carries on. Every `stream_chat` takes a
//! [`CancellationToken`] and races it against the next chunk, so a cancelled
//! answer stops between two chunks rather than after the next one. See
//! [`cancel`] for why that is a `watch` channel and not an `AtomicBool`.
//!
//! ## Example
//!
//! ```no_run
//! # use skia_lib::providers::{
//! #     ChatRequest, MockProvider, Provider, ProviderRole, Registry, CancellationToken,
//! # };
//! # use std::sync::Arc;
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use futures_util::StreamExt;
//!
//! let mut registry = Registry::new();
//! registry.register(Arc::new(MockProvider::new("offline")))?;
//! registry.route_all(vec!["offline".to_string()])?;
//!
//! let cancel = CancellationToken::new();
//! let mut answer = registry.stream_chat(
//!     ProviderRole::ChatFast,
//!     ChatRequest::user("what about the audio?"),
//!     cancel.clone(),
//! )?;
//!
//! while let Some(delta) = answer.next().await {
//!     print!("{}", delta?.content);
//!     if false {
//!         // Barge-in: the speaker carried on, so this answer is stale.
//!         cancel.cancel();
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod cancel;
mod mock;
mod openai;
mod registry;
mod sse;
mod types;
mod wire;

pub use cancel::CancellationToken;
pub use mock::{MockProvider, DEFAULT_ANSWER};
pub use openai::{OpenAiCompatible, OpenAiConfig};
pub use registry::Registry;
pub use sse::DeltaStream;
pub use types::{ApiKey, ChatMessage, ChatRequest, ChatRole, Delta, ProviderError, ProviderRole};

use futures_util::StreamExt;

/// Something that can answer a chat request, one fragment at a time.
///
/// Deliberately not `Debug`: an implementation may hold a credential, and a
/// derived `Debug` somewhere up the call stack is exactly how a key ends up in a
/// log. `Send + Sync` because a registry of these is shared across the overlay,
/// the live-meeting worker and the post-call pack.
pub trait Provider: Send + Sync {
    /// The user's own label for this provider, e.g. `groq-fast`. Named in every
    /// error it produces.
    fn id(&self) -> &str;

    /// The model it is configured to use, for the latency view to report.
    fn model(&self) -> &str;

    /// Stream an answer, abandoning it as soon as `cancel` trips.
    ///
    /// Nothing is sent until the returned stream is polled. Once the stream
    /// yields an `Err` it is finished; there are no further items.
    fn stream_chat(&self, request: ChatRequest, cancel: CancellationToken) -> DeltaStream;
}

/// Read a whole answer into one string.
///
/// For the places that genuinely need the finished text — the post-call summary,
/// a title, an email draft — and for tests. Streaming is still the primitive:
/// anything a user watches arrive should consume the deltas instead, because the
/// latency budget in `docs/ARCHITECTURE.md` is written in time to *first* token.
pub async fn collect_text(mut answer: DeltaStream) -> Result<String, ProviderError> {
    let mut text = String::new();
    while let Some(delta) = answer.next().await {
        text.push_str(&delta?.content);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn a_boxed_provider_answers_through_the_trait_object() {
        // The registry stores `Arc<dyn Provider>`, so this is the shape that has
        // to work, not the concrete type.
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::with_answer("offline", "one two"));

        assert_eq!(provider.id(), "offline");
        assert!(!provider.model().is_empty());
        assert_eq!(
            collect_text(provider.stream_chat(ChatRequest::user("go"), CancellationToken::new()))
                .await
                .expect("the mock must answer"),
            "one two"
        );
    }

    #[tokio::test]
    async fn collect_text_surfaces_a_failure_rather_than_returning_a_partial_answer() {
        let provider = MockProvider::new("offline");
        let error = collect_text(provider.stream_chat(
            ChatRequest {
                temperature: Some(f32::NAN),
                ..ChatRequest::user("go")
            },
            CancellationToken::new(),
        ))
        .await
        .expect_err("a broken request must not come back as text");

        assert!(error.to_string().contains("finite"), "{error}");
    }

    #[tokio::test]
    async fn the_registry_the_mock_and_the_decoder_work_end_to_end() {
        let mut registry = Registry::new();
        registry
            .register(Arc::new(MockProvider::new("offline")))
            .expect("registration must succeed");
        registry
            .route_all(vec!["offline".to_string()])
            .expect("routing must succeed");

        for role in ProviderRole::ALL {
            let answer = collect_text(
                registry
                    .stream_chat(role, ChatRequest::user("go"), CancellationToken::new())
                    .expect("every role is routed"),
            )
            .await
            .expect("the mock must answer");

            assert_eq!(
                answer, DEFAULT_ANSWER,
                "the {role} role must reassemble the canned answer exactly"
            );
        }
    }
}
