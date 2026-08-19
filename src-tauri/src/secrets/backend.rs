// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The seam between Skia and the operating system's credential store.
//!
//! [`SecretStore`](super::SecretStore) never talks to [`keyring`] directly; it
//! goes through [`Backend`]. That indirection buys two things:
//!
//! - **Tests must not touch a real keychain.** `keyring` 4 speaks to the actual
//!   macOS Keychain and Windows Credential Manager, so a unit test that used it
//!   would prompt for access, mutate the developer's login keychain, and leave
//!   entries behind. All the behaviour worth testing is exercised against
//!   [`MemoryBackend`] instead; the single test that does hit the real store is
//!   `#[ignore]`d.
//! - **"Missing" is not "broken".** [`BackendError::NotFound`] is its own
//!   variant so that an unconfigured provider can be told apart from a locked
//!   or denied keychain in code that can be read and tested. Collapsing the two
//!   would make a locked keychain look like "no key configured", and Skia would
//!   go on to send unauthenticated requests on the user's behalf.
//!
//! Nothing in this module ever formats a stored value — see [`classify`] for
//! why that takes deliberate effort rather than coming for free.

/// A place secrets can be kept.
///
/// Implementors receive the whole entry key. Skia identifies an entry by
/// `(service, account)` and mangles neither, so what a user sees in Keychain
/// Access or the Windows Credential Manager is exactly what is passed here.
pub(crate) trait Backend {
    /// Reads the secret stored for `(service, account)`.
    ///
    /// Returns [`BackendError::NotFound`] when no such entry exists. Deciding
    /// that this means "no key configured" is the caller's job, not this
    /// method's — see the module docs.
    fn get(&self, service: &str, account: &str) -> Result<String, BackendError>;

    /// Writes the secret for `(service, account)`, replacing any existing one.
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), BackendError>;

    /// Removes the entry for `(service, account)`.
    ///
    /// Returns [`BackendError::NotFound`] when there was nothing to remove.
    /// Making deletion idempotent is the caller's job.
    fn delete(&self, service: &str, account: &str) -> Result<(), BackendError>;
}

/// Why a [`Backend`] operation did not succeed.
///
/// The `String` carried by [`Unavailable`](BackendError::Unavailable) and
/// [`Failed`](BackendError::Failed) is a *description of a failure*, never a
/// stored value. [`classify`] is what upholds that for the real keychain, and
/// [`SecretError::from_backend`](super::SecretError::from_backend) scrubs the
/// string a second time on the write path in case a store quoted its own input
/// back at us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BackendError {
    /// There is no entry: one was never written, or it has been deleted.
    NotFound,

    /// The store could not be used at all — a locked keychain, access denied,
    /// or no credential store this platform supports.
    ///
    /// This is the variant that must never be mistaken for [`NotFound`]. The
    /// user has probably configured a key; we simply cannot see it right now.
    ///
    /// [`NotFound`]: BackendError::NotFound
    Unavailable(String),

    /// The store was reachable and the operation still failed.
    Failed(String),
}

/// The real thing: the OS credential store, via `keyring`.
///
/// A unit struct because `keyring` 4 keeps the platform store in a global that
/// it initialises on first use — there is no per-instance state worth holding,
/// and pretending otherwise would just invite someone to cache an [`Entry`]
/// across a keychain relock.
///
/// [`Entry`]: keyring::Entry
pub(crate) struct OsKeychain;

impl Backend for OsKeychain {
    fn get(&self, service: &str, account: &str) -> Result<String, BackendError> {
        entry(service, account)?.get_password().map_err(classify)
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), BackendError> {
        entry(service, account)?
            .set_password(secret)
            .map_err(classify)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), BackendError> {
        entry(service, account)?
            .delete_credential()
            .map_err(classify)
    }
}

/// Names an entry in the platform store.
///
/// Cheap, and deliberately not cached: the first call is what initialises the
/// platform store, and a keychain that was unlocked when Skia launched may not
/// be by the time the user configures a provider.
fn entry(service: &str, account: &str) -> Result<keyring::Entry, BackendError> {
    keyring::Entry::new(service, account).map_err(classify)
}

/// Converts a `keyring` error into a [`BackendError`], dropping anything that
/// could be a secret on the way through.
///
/// This function is the reason [`SecretError`](super::SecretError) does not
/// simply `#[from]` a `keyring::Error`. That type derives `Debug`, and two of
/// its variants carry the raw bytes of the credential that was just read:
/// `BadEncoding(Vec<u8>)` and `BadDataFormat(Vec<u8>, _)`. Its `Display` is
/// careful and hides them, but `Debug` is not, and `Debug` is what `dbg!`,
/// `unwrap`'s panic message, `#[derive(Debug)]` on an enclosing type, and most
/// structured loggers reach for. Wrapping the error would therefore put an API
/// key one `{:?}` away from a log file, which `SECURITY.md` calls out as the
/// highest-severity class of bug in this project.
///
/// Keeping it as a `#[source]` behind a hand-written `Debug` would not help
/// either: every error reporter worth using walks the source chain and
/// `Debug`-formats each link, so the payload would surface one hop down. Taking
/// the error **by value** here, and being the only place in Skia that does,
/// is what makes the guarantee structural rather than a matter of discipline.
/// No byte payload is formatted below.
///
/// The catch-all arm falls back to `Display`, not `Debug`, because
/// `keyring_core::Error` is `#[non_exhaustive]` and its authors clearly treat
/// `Display` as the redacted rendering — `BadEncoding` prints only "Password
/// data is not valid UTF-8". Leaning on that convention keeps a future variant
/// informative without printing its payload.
fn classify(err: keyring::Error) -> BackendError {
    use keyring::Error as K;

    match err {
        K::NoEntry => BackendError::NotFound,

        K::NoStorageAccess(source) => BackendError::Unavailable(format!(
            "the OS credential store could not be opened, which usually means it is \
             locked or access was denied: {source}"
        )),
        K::NoDefaultStore => BackendError::Unavailable(
            "this platform has no credential store that Skia can use".to_owned(),
        ),

        K::PlatformFailure(source) => {
            BackendError::Failed(format!("the OS credential store failed: {source}"))
        }

        // The payload here *is* the stored secret. Report its size and nothing
        // else; `{bytes:?}` would print the key as a list of byte values.
        K::BadEncoding(bytes) => BackendError::Failed(format!(
            "the stored value is not valid UTF-8 ({} bytes), so it was not written by Skia",
            bytes.len()
        )),
        // Same payload problem, plus a nested error that the store built while
        // it had the raw blob in hand. Withheld on purpose: a parse error is
        // entitled to quote what it failed to parse, and here that is the key.
        K::BadDataFormat(bytes, _source) => BackendError::Failed(format!(
            "the stored value ({} bytes) is not in the format this credential store writes; \
             the store's own explanation is withheld because it can quote the value",
            bytes.len()
        )),
        K::BadStoreFormat(reason) => {
            BackendError::Failed(format!("the credential store is malformed: {reason}"))
        }

        K::TooLong(attribute, limit) => BackendError::Failed(format!(
            "{attribute} is longer than this credential store's limit of {limit} characters"
        )),
        K::Invalid(parameter, reason) => BackendError::Failed(format!(
            "the credential store rejected {parameter}: {reason}"
        )),

        // `Display` for this variant formats the matching entries with `{:?}`.
        // Only the count is useful to a user anyway.
        K::Ambiguous(matches) => BackendError::Failed(format!(
            "{} stored credentials match this entry, so Skia will not guess between them",
            matches.len()
        )),
        K::NotSupportedByStore(reason) => BackendError::Failed(format!(
            "this credential store does not support the operation: {reason}"
        )),

        other => BackendError::Failed(format!("the OS credential store failed: {other}")),
    }
}

/// An in-memory [`Backend`] for tests.
///
/// Keyed by `(service, account)` exactly as the real store is, so tests can
/// assert that two providers — or two service names — never see each other's
/// keys.
///
/// Note that [`Call`] records which operation ran against which account and
/// deliberately *not* the secret involved: a test double that stashed keys in a
/// `Debug`-printable log would undo the property the tests exist to prove.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MemoryBackend {
    entries: std::sync::Mutex<std::collections::BTreeMap<(String, String), String>>,
    failure: Option<BackendError>,
    calls: std::sync::Mutex<Vec<Call>>,
}

/// One operation performed against a [`MemoryBackend`].
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Get(String),
    Set(String),
    Delete(String),
}

#[cfg(test)]
impl MemoryBackend {
    /// An empty store that behaves.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A store where every operation fails with `failure` — how a locked
    /// keychain, a denied prompt, or an unsupported platform is simulated.
    pub(crate) fn failing(failure: BackendError) -> Self {
        Self {
            failure: Some(failure),
            ..Self::default()
        }
    }

    /// Reads an entry without going through [`Backend`], so a test can check
    /// what was actually written (or that nothing was).
    pub(crate) fn stored(&self, service: &str, account: &str) -> Option<String> {
        self.lock_entries()
            .get(&(service.to_owned(), account.to_owned()))
            .cloned()
    }

    /// How many entries exist, across all services.
    pub(crate) fn len(&self) -> usize {
        self.lock_entries().len()
    }

    /// The operations performed so far, in order.
    pub(crate) fn calls(&self) -> Vec<Call> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn lock_entries(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<(String, String), String>> {
        match self.entries.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn record(&self, call: Call) {
        match self.calls.lock() {
            Ok(mut calls) => calls.push(call),
            Err(poisoned) => poisoned.into_inner().push(call),
        }
    }
}

#[cfg(test)]
impl Backend for MemoryBackend {
    fn get(&self, service: &str, account: &str) -> Result<String, BackendError> {
        self.record(Call::Get(account.to_owned()));
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        self.stored(service, account).ok_or(BackendError::NotFound)
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), BackendError> {
        self.record(Call::Set(account.to_owned()));
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        self.lock_entries()
            .insert((service.to_owned(), account.to_owned()), secret.to_owned());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), BackendError> {
        self.record(Call::Delete(account.to_owned()));
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        self.lock_entries()
            .remove(&(service.to_owned(), account.to_owned()))
            .map(|_| ())
            .ok_or(BackendError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, Backend, BackendError, Call, MemoryBackend};

    /// A value that must not appear in anything this module produces.
    const PLANTED: &str = "sk-planted-secret-do-not-log-9f3a1c";

    fn locked() -> keyring::Error {
        keyring::Error::NoStorageAccess(Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the default keychain is locked",
        )))
    }

    #[test]
    fn a_missing_entry_classifies_as_not_found() {
        assert_eq!(classify(keyring::Error::NoEntry), BackendError::NotFound);
    }

    #[test]
    fn a_locked_or_absent_store_classifies_as_unavailable_not_not_found() {
        // The whole point of the distinction: neither of these may look like
        // "the user has not configured a key".
        for err in [locked(), keyring::Error::NoDefaultStore] {
            let classified = classify(err);
            assert!(
                matches!(classified, BackendError::Unavailable(_)),
                "expected Unavailable, got {classified:?}"
            );
            assert_ne!(classified, BackendError::NotFound);
        }
    }

    #[test]
    fn the_reason_a_store_is_locked_survives_classification() {
        // Requirement 1 is only useful if the user can act on the error, so the
        // platform's explanation has to make it through.
        let BackendError::Unavailable(detail) = classify(locked()) else {
            panic!("a locked keychain must classify as Unavailable");
        };
        assert!(
            detail.contains("the default keychain is locked"),
            "the platform's reason was dropped: {detail}"
        );
    }

    #[test]
    fn bad_encoding_never_reveals_the_stored_bytes() {
        let classified = classify(keyring::Error::BadEncoding(PLANTED.as_bytes().to_vec()));
        let rendered = format!("{classified:?}|{classified:#?}");

        assert!(
            !rendered.contains(PLANTED),
            "the stored value leaked verbatim: {rendered}"
        );
        assert!(
            !rendered.contains(&format!("{:?}", PLANTED.as_bytes())),
            "the stored value leaked as a byte list: {rendered}"
        );
        assert!(
            rendered.contains(&format!("{} bytes", PLANTED.len())),
            "the length is the one safe detail and it is missing: {rendered}"
        );
    }

    #[test]
    fn bad_data_format_reveals_neither_the_bytes_nor_the_stores_explanation() {
        let classified = classify(keyring::Error::BadDataFormat(
            PLANTED.as_bytes().to_vec(),
            Box::new(std::io::Error::other(format!(
                "could not decrypt blob {PLANTED}"
            ))),
        ));
        let rendered = format!("{classified:?}|{classified:#?}");

        assert!(
            !rendered.contains(PLANTED),
            "the stored value leaked through the nested error: {rendered}"
        );
        assert!(
            !rendered.contains(&format!("{:?}", PLANTED.as_bytes())),
            "the stored value leaked as a byte list: {rendered}"
        );
    }

    #[test]
    fn ambiguous_entries_are_counted_not_printed() {
        let classified = classify(keyring::Error::Ambiguous(Vec::new()));
        let BackendError::Failed(detail) = classified else {
            panic!("an ambiguous entry is a real failure");
        };
        assert!(
            detail.contains('0') && detail.contains("match"),
            "expected a count of matching credentials: {detail}"
        );
    }

    #[test]
    fn the_double_keys_entries_by_service_and_account() {
        let backend = MemoryBackend::new();

        backend.set("svc.a", "openai", "key-a").expect("set");
        backend.set("svc.b", "openai", "key-b").expect("set");

        assert_eq!(backend.get("svc.a", "openai").as_deref(), Ok("key-a"));
        assert_eq!(backend.get("svc.b", "openai").as_deref(), Ok("key-b"));
        assert_eq!(backend.len(), 2);
    }

    #[test]
    fn the_double_reports_a_missing_entry_the_way_the_real_store_does() {
        let backend = MemoryBackend::new();

        assert_eq!(backend.get("svc", "openai"), Err(BackendError::NotFound));
        assert_eq!(backend.delete("svc", "openai"), Err(BackendError::NotFound));
        assert_eq!(
            backend.calls(),
            vec![
                Call::Get("openai".to_owned()),
                Call::Delete("openai".to_owned())
            ]
        );
    }

    #[test]
    fn an_injected_failure_applies_to_every_operation() {
        let failure = BackendError::Unavailable("locked".to_owned());
        let backend = MemoryBackend::failing(failure.clone());

        assert_eq!(backend.get("svc", "openai"), Err(failure.clone()));
        assert_eq!(backend.set("svc", "openai", "key"), Err(failure.clone()));
        assert_eq!(backend.delete("svc", "openai"), Err(failure));
        assert_eq!(backend.len(), 0, "a failing store must not have written");
    }
}
