// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Which provider serves which job.
//!
//! Skia is bring-your-own-key, so there is no house model and no sensible
//! default. One user runs everything through OpenRouter, another puts live
//! answers on Groq for the speed and Ask mode on a local Ollama, a third has
//! only a mock because they are working offline. Call sites cannot know any of
//! that, so they ask for a [`ProviderRole`] and the registry decides.
//!
//! ## Fallback is about configuration, not retries
//!
//! Each role holds an ordered list of provider ids and resolves to the first one
//! that is actually registered. That covers the case that really happens — a
//! role points at a provider whose key is not in the keychain on this machine,
//! so it should quietly use the next one — and it happens before a single byte
//! is sent.
//!
//! It deliberately does *not* retry a different provider after a failure
//! mid-answer. By then tokens are already on screen, and starting a second
//! provider would either duplicate them or contradict them. A failed answer is
//! reported as a failed answer.
//!
//! ## Nothing here touches the network
//!
//! Building a registry resolves no DNS, opens no connection and validates no
//! key. Startup cannot be held up by a provider the user may never use in this
//! session.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::cancel::CancellationToken;
use super::sse::DeltaStream;
use super::types::{ChatRequest, ProviderError, ProviderRole};
use super::Provider;

/// Named in configuration errors, which belong to the registry rather than to
/// any one provider.
const REGISTRY: &str = "the provider registry";

/// The configured providers and the roles they serve.
///
/// `Send + Sync`, so it can live in Tauri's managed state and be shared by the
/// overlay, the live-meeting worker and the post-call pack at once.
#[derive(Default)]
pub struct Registry {
    /// Sorted, so [`Registry::provider_ids`] is stable for a settings screen.
    providers: BTreeMap<String, Arc<dyn Provider>>,
    /// Role to ordered provider ids. An entry is never empty: [`Registry::route`]
    /// rejects that outright.
    routes: HashMap<ProviderRole, Vec<String>>,
}

impl Registry {
    /// An empty registry. Every role reports itself unconfigured until routed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a provider under its own id.
    ///
    /// A duplicate id is a configuration error rather than a silent replacement:
    /// two providers answering to one name means at least one of them is
    /// unreachable, and finding out later — by wondering why a role uses the
    /// wrong model — is much worse than finding out here.
    pub fn register(&mut self, provider: Arc<dyn Provider>) -> Result<(), ProviderError> {
        let id = provider.id().trim().to_string();

        if id.is_empty() {
            return Err(ProviderError::Config {
                provider: REGISTRY.to_string(),
                detail: "a provider cannot be registered with an empty id".to_string(),
            });
        }

        if self.providers.contains_key(&id) {
            return Err(ProviderError::DuplicateProvider { id });
        }

        self.providers.insert(id, provider);
        Ok(())
    }

    /// Point a role at an ordered list of provider ids, best first.
    ///
    /// The ids do not have to be registered yet, so configuration can be read in
    /// whatever order it arrives in. An id that is never registered simply gets
    /// skipped at resolution time, and if none of them resolve the error names
    /// every one that was tried.
    pub fn route(
        &mut self,
        role: ProviderRole,
        provider_ids: Vec<String>,
    ) -> Result<(), ProviderError> {
        if provider_ids.iter().all(|id| id.trim().is_empty()) {
            return Err(ProviderError::Config {
                provider: REGISTRY.to_string(),
                detail: format!(
                    "the '{role}' role was given no provider to fall back through, which \
                     leaves it unusable; leave it unrouted instead"
                ),
            });
        }

        self.routes.insert(
            role,
            provider_ids
                .into_iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect(),
        );
        Ok(())
    }

    /// Point every role at the same ordered list.
    ///
    /// The common case by a wide margin: one key, one provider, everything on it.
    pub fn route_all(&mut self, provider_ids: Vec<String>) -> Result<(), ProviderError> {
        for role in ProviderRole::ALL {
            self.route(role, provider_ids.clone())?;
        }
        Ok(())
    }

    /// The provider serving `role`.
    ///
    /// Walks the fallback list in order and returns the first registered
    /// provider. The two ways this fails are kept apart on purpose: a role with
    /// no route at all is a user who has not finished setting Skia up, while a
    /// role routed at providers that do not exist is a typo or a key that is
    /// missing from this machine's keychain. They need different fixes.
    pub fn resolve(&self, role: ProviderRole) -> Result<Arc<dyn Provider>, ProviderError> {
        let Some(order) = self.routes.get(&role) else {
            return Err(ProviderError::RoleNotConfigured { role });
        };

        for id in order {
            if let Some(provider) = self.providers.get(id) {
                return Ok(Arc::clone(provider));
            }
        }

        Err(ProviderError::RoleUnavailable {
            role,
            tried: order.clone(),
        })
    }

    /// Resolve `role` and start streaming, in one step.
    ///
    /// Resolution failures come back as an `Err` rather than as the stream's
    /// first item, because a role that is not configured is a settings problem
    /// and not a failed answer — the UI has somewhere different to put it.
    pub fn stream_chat(
        &self,
        role: ProviderRole,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<DeltaStream, ProviderError> {
        Ok(self.resolve(role)?.stream_chat(request, cancel))
    }

    /// One provider by id, whatever it is routed to.
    pub fn provider(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(id.trim()).map(Arc::clone)
    }

    /// Every registered id, in a stable order.
    pub fn provider_ids(&self) -> Vec<&str> {
        self.providers.keys().map(String::as_str).collect()
    }

    /// The fallback order for `role`, or an empty slice if it is unrouted.
    pub fn fallback_order(&self, role: ProviderRole) -> &[String] {
        self.routes.get(&role).map_or(&[], Vec::as_slice)
    }

    /// Which roles can actually be served right now.
    ///
    /// What a settings screen needs to say "live answers are ready, vision is
    /// not" without pretending to know why.
    pub fn ready_roles(&self) -> Vec<ProviderRole> {
        ProviderRole::ALL
            .into_iter()
            .filter(|role| self.resolve(*role).is_ok())
            .collect()
    }

    /// Whether anything at all is registered.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

/// Hand-written because `Arc<dyn Provider>` cannot be `Debug` — a provider
/// holding a credential must not be printable, even by accident.
impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let routes: BTreeMap<&str, &Vec<String>> = self
            .routes
            .iter()
            .map(|(role, ids)| (role.alias(), ids))
            .collect();

        f.debug_struct("Registry")
            .field("providers", &self.provider_ids())
            .field("routes", &routes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;
    use crate::providers::mock::MockProvider;

    /// A registry with three distinguishable mock providers registered but
    /// nothing routed.
    fn registered() -> Registry {
        let mut registry = Registry::new();
        for id in ["fast", "strict", "eyes"] {
            registry
                .register(Arc::new(MockProvider::with_answer(id, id)))
                .expect("a fresh id must register");
        }
        registry
    }

    /// The whole answer a role produces.
    async fn answer_for(registry: &Registry, role: ProviderRole) -> String {
        let mut stream = registry
            .stream_chat(role, ChatRequest::user("go"), CancellationToken::new())
            .expect("the role must resolve");

        let mut text = String::new();
        while let Some(delta) = stream.next().await {
            text.push_str(&delta.expect("a mock must not fail").content);
        }
        text
    }

    #[test]
    fn an_empty_registry_says_so_for_every_role() {
        let registry = Registry::new();

        assert!(registry.is_empty());
        assert!(registry.provider_ids().is_empty());
        assert!(registry.ready_roles().is_empty());

        for role in ProviderRole::ALL {
            let error = registry
                .resolve(role)
                // A provider is not `Debug` by design, so it is reduced to its
                // id before the assertion can ask to print it.
                .map(|provider| provider.id().to_string())
                .expect_err("an unconfigured role must not resolve to anything");
            assert!(
                matches!(error, ProviderError::RoleNotConfigured { role: got } if got == role),
                "unexpected error for {role}: {error}"
            );
            assert!(
                error.to_string().contains(role.alias()),
                "the error must name the role: {error}"
            );
            assert!(
                error
                    .to_string()
                    .contains("no model provider is configured"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_role_routed_at_nothing_registered_is_a_different_error() {
        let mut registry = registered();
        registry
            .route(
                ProviderRole::Vision,
                vec!["typo-provider".to_string(), "also-missing".to_string()],
            )
            .expect("routing at unknown ids is allowed; configuration order is not fixed");

        let error = registry
            .resolve(ProviderRole::Vision)
            .map(|provider| provider.id().to_string())
            .expect_err("a route to nothing must not resolve");

        match error {
            ProviderError::RoleUnavailable { role, ref tried } => {
                assert_eq!(role, ProviderRole::Vision);
                assert_eq!(tried, &["typo-provider", "also-missing"]);
            }
            other => panic!("expected RoleUnavailable, got {other}"),
        }
        assert!(
            error.to_string().contains("typo-provider"),
            "the error must name what it tried: {error}"
        );
    }

    #[tokio::test]
    async fn a_role_resolves_to_the_first_registered_provider_in_its_list() {
        let mut registry = registered();
        registry
            .route(
                ProviderRole::ChatFast,
                vec![
                    "not-on-this-machine".to_string(),
                    "fast".to_string(),
                    "strict".to_string(),
                ],
            )
            .unwrap();

        assert_eq!(
            registry.resolve(ProviderRole::ChatFast).unwrap().id(),
            "fast"
        );
        assert_eq!(
            answer_for(&registry, ProviderRole::ChatFast).await,
            "fast",
            "the resolved provider is the one that actually answers"
        );
    }

    #[tokio::test]
    async fn the_order_of_the_fallback_list_decides() {
        let mut registry = registered();

        registry
            .route(
                ProviderRole::ReasonStrict,
                vec!["strict".to_string(), "fast".to_string()],
            )
            .unwrap();
        assert_eq!(
            answer_for(&registry, ProviderRole::ReasonStrict).await,
            "strict"
        );

        // Same two providers, reversed. Nothing else changes.
        registry
            .route(
                ProviderRole::ReasonStrict,
                vec!["fast".to_string(), "strict".to_string()],
            )
            .unwrap();
        assert_eq!(
            answer_for(&registry, ProviderRole::ReasonStrict).await,
            "fast",
            "re-routing a role must take effect, and order must be what decides"
        );
    }

    #[test]
    fn roles_are_routed_independently() {
        let mut registry = registered();
        registry
            .route(ProviderRole::ChatFast, vec!["fast".to_string()])
            .unwrap();
        registry
            .route(ProviderRole::ReasonStrict, vec!["strict".to_string()])
            .unwrap();

        assert_eq!(
            registry.resolve(ProviderRole::ChatFast).unwrap().id(),
            "fast"
        );
        assert_eq!(
            registry.resolve(ProviderRole::ReasonStrict).unwrap().id(),
            "strict"
        );
        assert!(
            registry.resolve(ProviderRole::Vision).is_err(),
            "routing two roles must not quietly configure the third"
        );
        assert_eq!(
            registry.ready_roles(),
            [ProviderRole::ChatFast, ProviderRole::ReasonStrict]
        );
    }

    #[test]
    fn route_all_covers_every_role_at_once() {
        let mut registry = registered();
        registry.route_all(vec!["fast".to_string()]).unwrap();

        assert_eq!(registry.ready_roles(), ProviderRole::ALL);
        for role in ProviderRole::ALL {
            assert_eq!(registry.resolve(role).unwrap().id(), "fast");
            assert_eq!(registry.fallback_order(role), ["fast"]);
        }
    }

    #[test]
    fn an_empty_fallback_list_is_rejected() {
        let mut registry = registered();

        for empty in [Vec::new(), vec![String::new()], vec!["  ".to_string()]] {
            let error = registry
                .route(ProviderRole::ChatFast, empty)
                .expect_err("a role routed at nothing is worse than an unrouted role");
            assert!(error.to_string().contains("chat_fast"), "{error}");
            assert!(error.to_string().contains("no provider"), "{error}");
        }

        assert!(
            registry.fallback_order(ProviderRole::ChatFast).is_empty(),
            "a rejected route must not have been applied"
        );
    }

    #[test]
    fn blank_ids_are_trimmed_out_of_a_fallback_list() {
        let mut registry = registered();
        registry
            .route(
                ProviderRole::ChatFast,
                vec!["  ".to_string(), " fast ".to_string()],
            )
            .unwrap();

        assert_eq!(registry.fallback_order(ProviderRole::ChatFast), ["fast"]);
        assert_eq!(
            registry.resolve(ProviderRole::ChatFast).unwrap().id(),
            "fast"
        );
    }

    #[test]
    fn registering_the_same_id_twice_is_refused() {
        let mut registry = Registry::new();
        registry
            .register(Arc::new(MockProvider::new("groq")))
            .unwrap();

        let error = registry
            .register(Arc::new(MockProvider::new("groq")))
            .expect_err("one name must mean one provider");
        assert!(
            matches!(error, ProviderError::DuplicateProvider { ref id } if id == "groq"),
            "{error}"
        );

        assert_eq!(
            registry.provider_ids(),
            ["groq"],
            "the refused registration must not have been applied"
        );
    }

    #[test]
    fn a_provider_with_no_id_is_refused() {
        let mut registry = Registry::new();
        let error = registry
            .register(Arc::new(MockProvider::new("   ")))
            .expect_err("a nameless provider cannot be referred to");
        assert!(error.to_string().contains("empty id"), "{error}");
        assert!(registry.is_empty());
    }

    #[test]
    fn providers_can_be_looked_up_by_id() {
        let registry = registered();

        assert_eq!(registry.provider("strict").unwrap().id(), "strict");
        assert_eq!(
            registry.provider(" strict ").map(|p| p.id().to_string()),
            Some("strict".to_string()),
            "a pasted id with whitespace round it must still resolve"
        );
        assert!(registry.provider("nope").is_none());
        assert_eq!(
            registry.provider_ids(),
            ["eyes", "fast", "strict"],
            "ids come back sorted, so a settings screen does not reorder itself"
        );
    }

    #[test]
    fn the_debug_output_names_providers_without_printing_them() {
        let mut registry = registered();
        registry.route_all(vec!["fast".to_string()]).unwrap();

        let printed = format!("{registry:?}");
        assert!(printed.contains("fast"), "{printed}");
        assert!(printed.contains("chat_fast"), "{printed}");
        assert!(printed.contains("reason_strict"), "{printed}");
    }

    #[test]
    fn a_registry_is_shareable_across_threads() {
        // Guards the bound Tauri managed state needs. It is a compile-time
        // property, so the assertion is the call itself.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Registry>();
    }
}
