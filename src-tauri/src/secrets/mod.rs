// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! API keys, kept in the operating system's credential store.
//!
//! `SECURITY.md` states the requirement in one sentence: "keys are meant to
//! live in the OS keychain and never touch plaintext config, logs, or crash
//! reports". That is four separate obligations, and this is the only module in
//! Skia that is allowed to hold a key, so it is where all four are met.
//!
//! **Where keys live.** [`SecretStore`] is a thin, cheap handle over the
//! platform store — Keychain Services on macOS, the Credential Manager on
//! Windows. Nothing is cached, so a key that the user revokes in Keychain
//! Access is gone the next time Skia asks for it. Keys are never written to
//! [`Store`](crate::storage::Store): everything in `settings` is copied
//! verbatim into `export_json`, which is exactly the wrong place for a secret.
//!
//! **Why a key cannot reach a log.** No type here holds a key in a field that a
//! derived `Debug` can reach:
//!
//! - [`SecretStore`] holds a service name and nothing else.
//! - [`ApiKey`] hand-writes `Debug` to print `ApiKey(***)`, and deliberately
//!   implements no `Display` at all, so interpolating one is a compile error
//!   rather than a silent leak.
//! - [`SecretError`] carries descriptions of failures, never values. In
//!   particular it does *not* wrap `keyring::Error`, because two variants of
//!   that type carry the raw bytes of the credential just read and it derives
//!   `Debug`. See [`backend::classify`] for the full argument.
//! - Nothing here derives `Serialize`. An [`ApiKey`] cannot be serialised into
//!   a config file, an IPC payload, or a crash report by accident.
//!
//! **What is deliberately still missing.** A key read out of the store is a
//! plain `String`, so it lives on the heap until dropped and would appear in a
//! core dump taken mid-request. Closing that gap needs a zero-on-drop type
//! (`zeroize`, `secrecy`), which Skia does not depend on today.
//!
//! **Missing is not broken.** [`SecretStore::get_api_key`] answers `Ok(None)`
//! only when the store is working and holds no entry. A locked keychain or a
//! denied prompt is an `Err`. Those two cases *look* alike from the outside and
//! must never be conflated: treating a locked keychain as "no key configured"
//! would send the user's requests to a model provider unauthenticated. A
//! [`SecretError`] therefore never means "no key configured".

mod backend;

use std::fmt;

use backend::{Backend, BackendError, OsKeychain};

/// The real credential store, as a `'static` so [`SecretStore`] can borrow it
/// without allocating. [`OsKeychain`] is a unit struct with no state.
static OS_KEYCHAIN: OsKeychain = OsKeychain;

/// An API key, wrapped so it cannot be printed by accident.
///
/// This exists because `String` is far too easy to log. Wrap a key in this the
/// moment it is read and unwrap it only at the point of use:
///
/// ```
/// use skia_lib::secrets::ApiKey;
///
/// let key = ApiKey::new("sk-live-abc123");
/// assert_eq!(format!("{key:?}"), "ApiKey(***)");
/// assert_eq!(key.expose(), "sk-live-abc123");
/// ```
///
/// Three properties are load-bearing, and all three are the *absence* of
/// something:
///
/// - `Debug` is hand-written, so `{:?}`, `dbg!`, and a derived `Debug` on any
///   enclosing type print `ApiKey(***)`.
/// - There is no `Display`, so `format!("Bearer {key}")` does not compile. A
///   `Display` that printed `***` would have been worse than none: it would
///   turn a leak into a baffling authentication failure at runtime.
/// - There is no `Serialize`, so it cannot be written to a config file.
///
/// Reading the key requires naming [`expose`](ApiKey::expose), which is greppable
/// in review. Note that this guards against *printing* a key, not against a
/// core dump — see the module docs.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Hands out the key itself.
    ///
    /// Every call site is a place a key could escape into a log, so keep the
    /// result on the stack, pass it straight to the thing that needs it, and do
    /// not put it in a struct that derives `Debug`.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Prints `ApiKey(***)`. Never the key.
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// Everything that can go wrong storing or fetching an API key.
///
/// No variant means "there is no key" — that is [`Ok(None)`](Option::None) from
/// [`SecretStore::get_api_key`] and `Ok(false)` from
/// [`SecretStore::has_api_key`]. Every variant here is a real problem the user
/// has to know about.
///
/// `detail` is always a description of a failure and never a stored value. That
/// is enforced in two places: [`backend::classify`] refuses to format a
/// credential payload, and [`SecretError::from_backend`] scrubs the plaintext
/// back out on the write path.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// An empty key was offered. Storing it would be worse than refusing it:
    /// [`has_api_key`](SecretStore::has_api_key) would then report a configured
    /// provider whose every request fails to authenticate.
    #[error("the API key for '{provider_id}' is empty, and an empty key authenticates nothing")]
    EmptyKey { provider_id: String },

    /// The credential store could not be reached — locked keychain, denied
    /// access, or a platform with no store Skia can use.
    ///
    /// Distinct from "no key configured" on purpose. The user very likely does
    /// have a key stored; Skia just cannot see it at the moment.
    #[error(
        "the OS credential store is unavailable, so the API key for '{provider_id}' \
         could not be reached: {detail}"
    )]
    Unavailable { provider_id: String, detail: String },

    /// The credential store was reachable and the operation still failed.
    #[error("the OS credential store failed handling the API key for '{provider_id}': {detail}")]
    Backend { provider_id: String, detail: String },
}

impl SecretError {
    /// Turns a [`BackendError`] into the error Skia surfaces.
    ///
    /// `secret` is the plaintext the failed operation was carrying, if it was
    /// carrying one, and is used only to scrub it back out of `detail`. That
    /// covers the case where a credential store quotes its own *input* — a
    /// rejected parameter or a length limit is a natural place for a store to
    /// echo the value it was handed. It is a second line of defence, not the
    /// main one: stored payloads are excluded structurally by
    /// [`backend::classify`], which never formats them at all.
    fn from_backend(err: BackendError, provider_id: &str, secret: Option<&str>) -> Self {
        let provider_id = provider_id.to_owned();

        match err {
            // Every caller below turns NotFound into "no key" before reaching
            // here, so this arm is unreachable in practice. It is still an
            // error rather than a panic: a panic in a Tauri command would take
            // the request down, and a panic message is one of the places
            // SECURITY.md says a key must never appear.
            BackendError::NotFound => Self::Backend {
                provider_id,
                detail: "the credential store reported no such entry".to_owned(),
            },
            BackendError::Unavailable(detail) => Self::Unavailable {
                provider_id,
                detail: redact(detail, secret),
            },
            BackendError::Failed(detail) => Self::Backend {
                provider_id,
                detail: redact(detail, secret),
            },
        }
    }
}

/// Replaces `secret` with `***` wherever it appears in a store-supplied string.
///
/// The empty-secret guard matters: `str::replace` with an empty pattern matches
/// at every character boundary and would mangle the whole message.
fn redact(detail: String, secret: Option<&str>) -> String {
    match secret {
        Some(secret) if !secret.is_empty() => detail.replace(secret, "***"),
        _ => detail,
    }
}

/// The store's operations, over any [`Backend`].
///
/// [`SecretStore`] is a handle on the real keychain and holds no backend, so
/// the policy lives here where a test can drive it against an in-memory double.
/// Three decisions worth reading as code rather than taking on trust:
/// `NotFound` becoming `Ok(None)`, `NotFound` making deletion idempotent, and
/// every other failure staying an `Err`.
struct Vault<'a, B: Backend> {
    service: &'a str,
    backend: &'a B,
}

impl<B: Backend> Vault<'_, B> {
    fn set_api_key(&self, provider_id: &str, key: &str) -> Result<(), SecretError> {
        // Checked before the store is touched, so a rejected key is never
        // written and then rolled back.
        if key.is_empty() {
            return Err(SecretError::EmptyKey {
                provider_id: provider_id.to_owned(),
            });
        }

        self.backend
            .set(self.service, provider_id, key)
            .map_err(|err| SecretError::from_backend(err, provider_id, Some(key)))
    }

    fn get_api_key(&self, provider_id: &str) -> Result<Option<String>, SecretError> {
        match self.backend.get(self.service, provider_id) {
            Ok(key) => Ok(Some(key)),
            // The only failure that means "the user has not configured this
            // provider". Everything else has to reach the caller as an error.
            Err(BackendError::NotFound) => Ok(None),
            Err(err) => Err(SecretError::from_backend(err, provider_id, None)),
        }
    }

    fn delete_api_key(&self, provider_id: &str) -> Result<(), SecretError> {
        match self.backend.delete(self.service, provider_id) {
            // Idempotent: the caller asked for the key to be gone, and it is.
            Ok(()) | Err(BackendError::NotFound) => Ok(()),
            Err(err) => Err(SecretError::from_backend(err, provider_id, None)),
        }
    }

    fn has_api_key(&self, provider_id: &str) -> Result<bool, SecretError> {
        // Built on get_api_key so the NotFound handling cannot drift apart, and
        // so a locked keychain reports an error instead of `false`. The key
        // itself is dropped at the end of this expression and never returned.
        self.get_api_key(provider_id).map(|key| key.is_some())
    }
}

/// Stores API keys in the OS credential store, one per provider.
///
/// An entry is identified by `(service, provider_id)`, which is the primary key
/// the platform stores use, so `SecretStore::new("com.skia.app")` puts a
/// user-visible item named `com.skia.app` with account `openai` in Keychain
/// Access. Pick one service name for the whole app and keep it stable; changing
/// it orphans every key the user has already entered.
///
/// Cheap to construct and to clone — the handle holds a service name, never a
/// key.
///
/// ```no_run
/// use skia_lib::secrets::SecretStore;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let store = SecretStore::new("com.skia.app");
/// store.set_api_key("openai", "sk-live-abc123")?;
///
/// // `false` here means the user has not configured a key. It never means the
/// // keychain was locked -- that is an `Err`.
/// assert!(store.has_api_key("openai")?);
///
/// store.delete_api_key("openai")?;
/// assert_eq!(store.get_api_key("openai")?, None);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct SecretStore {
    service: String,
}

impl SecretStore {
    /// Creates a handle for a service name. Touches no keychain.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Stores the API key for a provider id (`"openai"`, `"groq"`), replacing
    /// any key already stored for it.
    ///
    /// # Errors
    ///
    /// [`SecretError::EmptyKey`] if `key` is empty — nothing is written.
    /// [`SecretError::Unavailable`] if the credential store cannot be reached,
    /// and [`SecretError::Backend`] if it refuses the write.
    pub fn set_api_key(&self, provider_id: &str, key: &str) -> Result<(), SecretError> {
        self.vault().set_api_key(provider_id, key)
    }

    /// Reads the API key for a provider id.
    ///
    /// `Ok(None)` means the credential store is working and holds no key for
    /// this provider. It never means the store could not be read: see the
    /// module docs for why that distinction is worth the extra variant.
    ///
    /// # Errors
    ///
    /// [`SecretError::Unavailable`] if the store is locked or access was
    /// denied, and [`SecretError::Backend`] for any other failure.
    pub fn get_api_key(&self, provider_id: &str) -> Result<Option<String>, SecretError> {
        self.vault().get_api_key(provider_id)
    }

    /// Removes the API key for a provider id.
    ///
    /// Idempotent: deleting a key that is not there is `Ok(())`, so this is safe
    /// to call when disconnecting a provider whose state the caller is unsure
    /// of.
    ///
    /// # Errors
    ///
    /// [`SecretError::Unavailable`] or [`SecretError::Backend`] if the store
    /// could not be reached or refused the deletion — in which case the key may
    /// still be there, and saying so is the point.
    pub fn delete_api_key(&self, provider_id: &str) -> Result<(), SecretError> {
        self.vault().delete_api_key(provider_id)
    }

    /// Whether a key is stored for a provider id.
    ///
    /// For UI that has to show a provider as configured without ever handling
    /// the key: this returns a `bool`, so there is no key for the frontend to
    /// leak. The key is read from the store to answer the question and dropped
    /// immediately.
    ///
    /// # Errors
    ///
    /// Never returns `Ok(false)` for a keychain it could not read — that is
    /// [`SecretError::Unavailable`]. `Ok(false)` means, and only means, that the
    /// user has not configured this provider.
    pub fn has_api_key(&self, provider_id: &str) -> Result<bool, SecretError> {
        self.vault().has_api_key(provider_id)
    }

    fn vault(&self) -> Vault<'_, OsKeychain> {
        Vault {
            service: &self.service,
            backend: &OS_KEYCHAIN,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::backend::{Call, MemoryBackend};
    use super::{ApiKey, BackendError, SecretError, SecretStore, Vault};

    /// A value that must not show up in any rendering of any type in this
    /// module. Distinctive enough that a `contains` check is meaningful.
    const PLANTED: &str = "sk-planted-secret-do-not-log-9f3a1c";

    const SERVICE: &str = "dev.skia.secrets.unit-test";

    fn vault(backend: &MemoryBackend) -> Vault<'_, MemoryBackend> {
        Vault {
            service: SERVICE,
            backend,
        }
    }

    /// A store that is locked or denying access — the case that must never be
    /// mistaken for "no key configured".
    fn locked() -> MemoryBackend {
        MemoryBackend::failing(BackendError::Unavailable(
            "the default keychain is locked".to_owned(),
        ))
    }

    /// Everything a caller could print about an error, including the whole
    /// source chain, since error reporters walk it.
    fn render(err: &SecretError) -> String {
        let mut rendered = format!("{err}|{err:?}|{err:#?}");
        let mut source = err.source();
        while let Some(link) = source {
            rendered.push_str(&format!("|{link}|{link:?}"));
            source = link.source();
        }
        rendered
    }

    // --- storing and reading -------------------------------------------------

    #[test]
    fn a_stored_key_reads_back() {
        let backend = MemoryBackend::new();

        vault(&backend)
            .set_api_key("openai", PLANTED)
            .expect("storing a key");

        assert_eq!(
            vault(&backend)
                .get_api_key("openai")
                .expect("reading a key"),
            Some(PLANTED.to_owned())
        );
        assert!(vault(&backend).has_api_key("openai").expect("has"));
    }

    #[test]
    fn storing_a_key_replaces_the_previous_one() {
        let backend = MemoryBackend::new();
        let vault = vault(&backend);

        vault.set_api_key("openai", "first").expect("first write");
        vault.set_api_key("openai", "second").expect("second write");

        assert_eq!(
            vault.get_api_key("openai").expect("read"),
            Some("second".to_owned())
        );
        assert_eq!(backend.len(), 1, "a replacement must not add an entry");
    }

    #[test]
    fn providers_do_not_see_each_others_keys() {
        let backend = MemoryBackend::new();
        let vault = vault(&backend);

        vault.set_api_key("openai", "key-openai").expect("write");
        vault.set_api_key("groq", "key-groq").expect("write");

        assert_eq!(
            vault.get_api_key("openai").expect("read"),
            Some("key-openai".to_owned())
        );
        assert_eq!(
            vault.get_api_key("groq").expect("read"),
            Some("key-groq".to_owned())
        );
        assert_eq!(vault.get_api_key("anthropic").expect("read"), None);
    }

    #[test]
    fn service_names_are_isolated_from_each_other() {
        // Two service names are two separate namespaces in the platform store,
        // which is what keeps a stray SecretStore from reading Skia's keys.
        let backend = MemoryBackend::new();

        Vault {
            service: "dev.skia.one",
            backend: &backend,
        }
        .set_api_key("openai", "key-one")
        .expect("write");

        let other = Vault {
            service: "dev.skia.two",
            backend: &backend,
        };
        assert_eq!(other.get_api_key("openai").expect("read"), None);
        assert!(!other.has_api_key("openai").expect("has"));
    }

    // --- missing is not broken ----------------------------------------------

    #[test]
    fn a_provider_with_no_key_reads_as_none_not_an_error() {
        let backend = MemoryBackend::new();

        assert_eq!(vault(&backend).get_api_key("openai").expect("read"), None);
        assert!(!vault(&backend).has_api_key("openai").expect("has"));
    }

    #[test]
    fn a_locked_keychain_is_an_error_not_a_missing_key() {
        // The requirement this module exists to satisfy. If this ever returns
        // Ok(None), Skia starts sending unauthenticated requests to a provider
        // the user did configure a key for.
        let backend = locked();

        let err = vault(&backend)
            .get_api_key("openai")
            .expect_err("a locked keychain must not read as Ok(None)");
        assert!(
            matches!(err, SecretError::Unavailable { .. }),
            "a locked keychain must be reported as unavailable, got {err:?}"
        );
    }

    #[test]
    fn has_api_key_reports_an_error_rather_than_false_when_the_keychain_is_locked() {
        // The same trap one level up: a `bool` return makes `false` the tempting
        // thing to answer, and `false` would render the provider as
        // unconfigured in the UI while the user's key sits in the keychain.
        let backend = locked();

        let err = vault(&backend)
            .has_api_key("openai")
            .expect_err("a locked keychain must not read as Ok(false)");
        assert!(
            matches!(err, SecretError::Unavailable { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn a_locked_keychain_is_an_error_when_storing_or_deleting_too() {
        let backend = locked();

        assert!(vault(&backend).set_api_key("openai", PLANTED).is_err());
        assert!(
            vault(&backend).delete_api_key("openai").is_err(),
            "deletion is idempotent for a missing key, not for an unreachable store"
        );
    }

    #[test]
    fn a_reachable_store_that_fails_is_reported_as_a_backend_error() {
        let backend = MemoryBackend::failing(BackendError::Failed("disk is full".to_owned()));

        let err = vault(&backend)
            .set_api_key("openai", PLANTED)
            .expect_err("a failing store must not report success");
        assert!(matches!(err, SecretError::Backend { .. }), "got {err:?}");
    }

    // --- deleting ------------------------------------------------------------

    #[test]
    fn deleting_a_key_that_is_not_there_succeeds() {
        let backend = MemoryBackend::new();

        vault(&backend)
            .delete_api_key("openai")
            .expect("deleting a missing key is not a failure");
    }

    #[test]
    fn deleting_is_idempotent_and_actually_removes_the_key() {
        let backend = MemoryBackend::new();
        let vault = vault(&backend);

        vault.set_api_key("openai", PLANTED).expect("write");
        vault.delete_api_key("openai").expect("first delete");
        vault.delete_api_key("openai").expect("second delete");

        assert_eq!(vault.get_api_key("openai").expect("read"), None);
        assert_eq!(backend.stored(SERVICE, "openai"), None);
    }

    // --- empty keys ----------------------------------------------------------

    #[test]
    fn an_empty_key_is_refused_and_not_written() {
        let backend = MemoryBackend::new();

        let err = vault(&backend)
            .set_api_key("openai", "")
            .expect_err("an empty key must be refused");
        assert!(matches!(err, SecretError::EmptyKey { .. }), "got {err:?}");

        assert_eq!(
            backend.len(),
            0,
            "an empty key must not reach the credential store at all"
        );
        assert_eq!(backend.calls(), Vec::new());
    }

    #[test]
    fn refusing_an_empty_key_leaves_an_existing_key_alone() {
        let backend = MemoryBackend::new();
        let vault = vault(&backend);

        vault.set_api_key("openai", PLANTED).expect("write");
        vault
            .set_api_key("openai", "")
            .expect_err("an empty key must be refused");

        assert_eq!(
            vault.get_api_key("openai").expect("read"),
            Some(PLANTED.to_owned()),
            "a refused write must not have clobbered the stored key"
        );
    }

    // --- keys cannot be printed ---------------------------------------------

    #[test]
    fn api_key_debug_prints_a_placeholder() {
        let key = ApiKey::new(PLANTED);

        assert_eq!(format!("{key:?}"), "ApiKey(***)");
        assert_eq!(format!("{key:#?}"), "ApiKey(***)");
        assert!(!format!("{key:?}").contains(PLANTED));
        assert_eq!(key.expose(), PLANTED, "the key is still retrievable");
    }

    #[test]
    fn api_key_hides_the_secret_inside_an_enclosing_derived_debug() {
        // The realistic leak: a config struct that derives Debug and gets
        // logged. This is what the hand-written Debug is for.
        #[derive(Debug)]
        struct ProviderConfig {
            id: &'static str,
            key: ApiKey,
        }

        let config = ProviderConfig {
            id: "openai",
            key: ApiKey::new(PLANTED),
        };
        let rendered = format!("{config:?}|{config:#?}");

        assert!(!rendered.contains(PLANTED), "the key leaked: {rendered}");
        assert!(rendered.contains("ApiKey(***)"));
        assert!(rendered.contains("openai"), "non-secret fields still print");

        // The struct really is holding the key -- it is hidden, not absent.
        assert_eq!(config.id, "openai");
        assert_eq!(config.key.expose(), PLANTED);
    }

    #[test]
    fn the_store_handle_never_holds_a_key() {
        let store = SecretStore::new(SERVICE);
        let rendered = format!("{store:?}|{store:#?}");

        assert!(!rendered.contains(PLANTED), "the key leaked: {rendered}");
        assert!(
            rendered.contains(SERVICE),
            "the service name is not a secret and is worth printing"
        );
    }

    #[test]
    fn no_error_reveals_the_key_even_when_the_store_echoes_it_back() {
        // A credential store is entitled to quote the value it rejected. If one
        // does, that string must not survive into an error message.
        for failure in [
            BackendError::Unavailable(format!("locked while writing {PLANTED}")),
            BackendError::Failed(format!("rejected value {PLANTED}: too long")),
        ] {
            let backend = MemoryBackend::failing(failure);

            let err = vault(&backend)
                .set_api_key("openai", PLANTED)
                .expect_err("the store was told to fail");
            let rendered = render(&err);

            assert!(!rendered.contains(PLANTED), "the key leaked: {rendered}");
            assert!(rendered.contains("***"), "expected a redaction: {rendered}");
            assert!(
                rendered.contains("openai"),
                "the provider id is not a secret and is needed to act on this"
            );
        }
    }

    #[test]
    fn no_error_variant_renders_the_key() {
        let backend = locked();
        let vault = vault(&backend);

        let errors = [
            vault.set_api_key("openai", "").expect_err("empty key"),
            vault
                .set_api_key("openai", PLANTED)
                .expect_err("locked set"),
            vault.get_api_key("openai").expect_err("locked get"),
            vault.delete_api_key("openai").expect_err("locked delete"),
            vault.has_api_key("openai").expect_err("locked has"),
            SecretError::from_backend(BackendError::NotFound, "openai", Some(PLANTED)),
        ];

        for err in &errors {
            let rendered = render(err);
            assert!(!rendered.contains(PLANTED), "the key leaked: {rendered}");
        }
    }

    #[test]
    fn errors_carry_no_source_chain_to_leak_through() {
        // SecretError translates keyring errors rather than wrapping them: two
        // keyring variants carry the raw credential bytes and that type derives
        // Debug, so a `#[source]` link would put the key one hop from any error
        // reporter that walks the chain.
        let backend = locked();
        let err = vault(&backend).get_api_key("openai").expect_err("locked");

        assert!(
            err.source().is_none(),
            "a source chain here would be a leak vector"
        );
    }

    // --- reading is read-only ------------------------------------------------

    #[test]
    fn has_api_key_only_reads() {
        let backend = MemoryBackend::new();
        vault(&backend)
            .set_api_key("openai", PLANTED)
            .expect("write");

        let before = backend.calls().len();
        vault(&backend).has_api_key("openai").expect("has");

        assert_eq!(
            backend.calls()[before..],
            [Call::Get("openai".to_owned())],
            "checking for a key must not write to or delete from the store"
        );
    }

    // --- the real credential store ------------------------------------------

    /// Round-trips a key through the actual OS credential store.
    ///
    /// Ignored, so it never runs in CI and never runs by accident: it writes to
    /// the developer's real login keychain, and on an unsigned build macOS may
    /// put up an access prompt. Run it deliberately, on a machine with an
    /// unlocked keychain:
    ///
    /// ```text
    /// cargo test --manifest-path src-tauri/Cargo.toml secrets -- --ignored --nocapture
    /// ```
    ///
    /// The service name is a throwaway that no Skia build uses, so a failure
    /// mid-test cannot corrupt a real configuration. Results are collected
    /// before anything is asserted, so an assertion failure still cannot leave
    /// an entry behind.
    ///
    /// What this adds over the tests above: it proves `NoEntry` is the only
    /// thing the real backend maps to `Ok(None)`. The complementary half — that
    /// a locked or denied store maps to `Unavailable` — is proved by
    /// `backend::tests`, which feeds real `keyring::Error` values through the
    /// real `classify`, because a test cannot lock the developer's keychain.
    #[test]
    #[ignore = "writes to the real OS keychain; run manually with --ignored"]
    fn real_os_keychain_round_trip() {
        const THROWAWAY_SERVICE: &str = "dev.skia.secrets.throwaway-integration-test";
        const PROVIDER: &str = "skia-throwaway-provider";
        const KEY: &str = "skia-test-value-not-a-real-key";

        let store = SecretStore::new(THROWAWAY_SERVICE);

        // Start from a known state in case a previous run was interrupted.
        let cleaned = store.delete_api_key(PROVIDER);
        let missing_before = store.get_api_key(PROVIDER);
        let has_before = store.has_api_key(PROVIDER);

        let written = store.set_api_key(PROVIDER, KEY);
        let read_back = store.get_api_key(PROVIDER);
        let has_after = store.has_api_key(PROVIDER);

        let deleted = store.delete_api_key(PROVIDER);
        let deleted_again = store.delete_api_key(PROVIDER);
        let missing_after = store.get_api_key(PROVIDER);

        cleaned.expect("clearing any leftover entry");
        assert_eq!(missing_before.expect("read before write"), None);
        assert!(!has_before.expect("has before write"));

        written.expect("writing to the real keychain");
        assert_eq!(read_back.expect("read after write"), Some(KEY.to_owned()));
        assert!(has_after.expect("has after write"));

        deleted.expect("deleting from the real keychain");
        deleted_again.expect("deletion must be idempotent against the real store");
        assert_eq!(
            missing_after.expect("read after delete"),
            None,
            "the entry must be gone, not merely unreadable"
        );
    }
}
