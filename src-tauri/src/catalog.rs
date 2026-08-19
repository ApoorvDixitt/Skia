// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The bring-your-own-key provider catalog.
//!
//! Skia has no backend and ships no credentials. Every entry here is something
//! the user supplies a key for (or runs locally), and the key lives in the OS
//! keychain — see [`crate::secrets`].
//!
//! Every provider is reached through one OpenAI-compatible client. That is not a
//! simplification: OpenAI, Groq, Cerebras, OpenRouter, Ollama, and LM Studio
//! speak that shape natively, and Anthropic and Google both publish an
//! OpenAI-compatible endpoint alongside their own APIs. One client means one SSE
//! parser to get right and one place for streaming bugs to live.
//!
//! Model ids are defaults, not a fixed list. They go stale as vendors ship new
//! models, so the user can override the model per provider; nothing here is
//! load-bearing beyond giving a working first run.

use serde::{Deserialize, Serialize};

use crate::providers::ProviderRole;

/// Where a provider runs, which decides whether a key is needed at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Hosting {
    /// A remote service. Needs a key the user supplies.
    Cloud,
    /// Runs on this machine. No key, no cost, nothing leaves the device.
    Local,
    /// Deterministic offline output for development. Not a model.
    Mock,
}

/// One catalog entry.
///
/// `Serialize` only: the catalog is compiled in, so it is sent to the UI but
/// never read back from anywhere.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    /// Stable id. Also the keychain account name, so it must not change.
    pub id: &'static str,
    pub label: &'static str,
    pub hosting: Hosting,
    /// Base URL up to but excluding `/chat/completions`.
    pub base_url: &'static str,
    /// Default model for each role this provider is a sensible choice for.
    pub default_model: &'static str,
    /// Roles this provider is a good default for, per the product's needs:
    /// fast first-token for live answers, stronger reasoning for Ask, vision
    /// for region capture.
    pub roles: &'static [ProviderRole],
    /// Where the user gets a key. Shown in settings; never fetched.
    pub api_key_url: Option<&'static str>,
    /// Honest one-liner about the trade-off, shown in the picker.
    pub note: &'static str,
}

impl CatalogEntry {
    /// Whether the user must supply a key before this is usable.
    pub fn needs_api_key(&self) -> bool {
        matches!(self.hosting, Hosting::Cloud)
    }
}

/// The shipped catalog.
///
/// Ordered roughly by how useful each is for a first run: the local and free
/// options first, because the PRD's promise is that Skia costs nothing to run.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "mock",
        label: "Mock (offline)",
        hosting: Hosting::Mock,
        base_url: "",
        default_model: "mock-1",
        roles: &[
            ProviderRole::ChatFast,
            ProviderRole::ReasonStrict,
            ProviderRole::Vision,
        ],
        api_key_url: None,
        note: "Canned test output, not a model. Lets the app be exercised with no key and no network.",
    },
    CatalogEntry {
        id: "ollama",
        label: "Ollama (local)",
        hosting: Hosting::Local,
        base_url: "http://localhost:11434/v1",
        default_model: "qwen3:8b",
        roles: &[ProviderRole::ChatFast, ProviderRole::ReasonStrict],
        api_key_url: None,
        note: "Runs on this machine. Free, private, nothing leaves the device. Needs Ollama running and the model pulled.",
    },
    CatalogEntry {
        id: "lmstudio",
        label: "LM Studio (local)",
        hosting: Hosting::Local,
        base_url: "http://localhost:1234/v1",
        default_model: "local-model",
        roles: &[ProviderRole::ChatFast, ProviderRole::ReasonStrict],
        api_key_url: None,
        note: "Runs on this machine via LM Studio's local server. Set the model to whatever you have loaded.",
    },
    CatalogEntry {
        id: "groq",
        label: "Groq",
        hosting: Hosting::Cloud,
        base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.3-70b-versatile",
        roles: &[ProviderRole::ChatFast],
        api_key_url: Some("https://console.groq.com/keys"),
        note: "Fastest time to first token, which is what live answers are judged on.",
    },
    CatalogEntry {
        id: "cerebras",
        label: "Cerebras",
        hosting: Hosting::Cloud,
        base_url: "https://api.cerebras.ai/v1",
        default_model: "llama-3.3-70b",
        roles: &[ProviderRole::ChatFast],
        api_key_url: Some("https://cloud.cerebras.ai"),
        note: "Very high throughput. Good for long or agentic answers.",
    },
    CatalogEntry {
        id: "openai",
        label: "OpenAI",
        hosting: Hosting::Cloud,
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-5",
        roles: &[ProviderRole::ReasonStrict, ProviderRole::Vision],
        api_key_url: Some("https://platform.openai.com/api-keys"),
        note: "Strong general reasoning and vision.",
    },
    CatalogEntry {
        id: "anthropic",
        label: "Anthropic",
        hosting: Hosting::Cloud,
        // Anthropic's OpenAI-compatible surface, so it shares this client.
        base_url: "https://api.anthropic.com/v1",
        default_model: "claude-sonnet-4-5",
        roles: &[ProviderRole::ReasonStrict, ProviderRole::Vision],
        api_key_url: Some("https://console.anthropic.com/settings/keys"),
        note: "Strong reasoning and long context. Uses Anthropic's OpenAI-compatible endpoint.",
    },
    CatalogEntry {
        id: "gemini",
        label: "Google Gemini",
        hosting: Hosting::Cloud,
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        default_model: "gemini-2.5-flash",
        roles: &[ProviderRole::ChatFast, ProviderRole::Vision],
        api_key_url: Some("https://aistudio.google.com/apikey"),
        note: "Fast and inexpensive, with good vision. Uses Google's OpenAI-compatible endpoint.",
    },
    CatalogEntry {
        id: "openrouter",
        label: "OpenRouter",
        hosting: Hosting::Cloud,
        base_url: "https://openrouter.ai/api/v1",
        default_model: "anthropic/claude-sonnet-4.5",
        roles: &[
            ProviderRole::ChatFast,
            ProviderRole::ReasonStrict,
            ProviderRole::Vision,
        ],
        api_key_url: Some("https://openrouter.ai/keys"),
        note: "One key for many models. Simplest way to try several without separate accounts.",
    },
];

/// Looks up an entry by id.
pub fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// Default provider ids for a role, in preference order — the fallback chain.
pub fn defaults_for_role(role: ProviderRole) -> Vec<&'static str> {
    CATALOG
        .iter()
        .filter(|e| e.roles.contains(&role))
        .map(|e| e.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "catalog ids must be unique: {ids:?}");
    }

    #[test]
    fn cloud_providers_tell_the_user_where_to_get_a_key() {
        for e in CATALOG {
            if e.hosting == Hosting::Cloud {
                assert!(
                    e.api_key_url.is_some(),
                    "{} needs a key but offers no link to get one",
                    e.id
                );
                assert!(e.needs_api_key(), "{} is cloud so it needs a key", e.id);
            } else {
                assert!(!e.needs_api_key(), "{} must not demand a key", e.id);
            }
        }
    }

    #[test]
    fn local_and_mock_providers_never_require_a_key() {
        // The product promise is that Skia can run for free. If this breaks,
        // the zero-cost path is gone.
        let free: Vec<&str> = CATALOG
            .iter()
            .filter(|e| !e.needs_api_key())
            .map(|e| e.id)
            .collect();
        assert!(
            free.contains(&"ollama"),
            "a local, keyless option must exist: {free:?}"
        );
    }

    #[test]
    fn cloud_base_urls_are_https_and_local_ones_are_loopback() {
        for e in CATALOG {
            match e.hosting {
                Hosting::Cloud => assert!(
                    e.base_url.starts_with("https://"),
                    "{} sends a key, so it must use TLS: {}",
                    e.id,
                    e.base_url
                ),
                Hosting::Local => assert!(
                    e.base_url.contains("localhost") || e.base_url.contains("127.0.0.1"),
                    "{} claims to be local but points off-machine: {}",
                    e.id,
                    e.base_url
                ),
                Hosting::Mock => assert!(e.base_url.is_empty()),
            }
        }
    }

    #[test]
    fn base_urls_do_not_include_the_chat_completions_path() {
        // The client appends it; a doubled path is a confusing 404.
        for e in CATALOG {
            assert!(
                !e.base_url.contains("chat/completions"),
                "{} must be a base URL only",
                e.id
            );
            assert!(
                !e.base_url.ends_with('/'),
                "{} must not have a trailing slash",
                e.id
            );
        }
    }

    #[test]
    fn every_role_has_at_least_one_default_beyond_the_mock() {
        for role in ProviderRole::ALL {
            let ids = defaults_for_role(role);
            assert!(
                ids.iter().any(|id| *id != "mock"),
                "role {} has no real provider: {ids:?}",
                role.alias()
            );
        }
    }

    #[test]
    fn the_mock_is_first_so_a_keyless_first_run_still_works() {
        assert_eq!(CATALOG.first().map(|e| e.id), Some("mock"));
    }

    #[test]
    fn lookup_finds_entries_and_rejects_unknown_ids() {
        assert!(entry("groq").is_some());
        assert!(entry("definitely-not-a-provider").is_none());
    }
}
