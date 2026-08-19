// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Cancellation for in-flight generation.
//!
//! Barge-in is the whole reason this exists. In a live meeting Skia starts
//! answering speculatively, and the moment the speaker carries on the answer is
//! stale — the roadmap lists "speculative retrieval and generation, cancelled
//! on barge-in" as a P0. A stale generation that keeps streaming does not just
//! waste tokens, it puts the wrong words on screen.
//!
//! An `AtomicBool` would be the obvious implementation, and it is what a
//! polling loop wants. It is the wrong shape here: a generation spends nearly
//! all of its time parked on the next network chunk, so a flag is only noticed
//! once the *provider* speaks again — which can be hundreds of milliseconds
//! after the user did. A [`tokio::sync::watch`] channel gives the same
//! cheap boolean plus something to await, so a cancelled stream can be dropped
//! between two chunks instead of after the next one. `tokio-util`'s
//! `CancellationToken` does exactly this; it is not a dependency of this crate
//! and one boolean channel is not worth adding it for.

use std::sync::Arc;

use tokio::sync::watch;

/// A shared "stop now" signal.
///
/// Cloning it shares the signal rather than copying it, so the audio thread
/// that detects barge-in and the stream that has to stop can hold the same
/// token. Cancellation is one-way and idempotent: a token that has been
/// cancelled stays cancelled, because a generation that was abandoned must
/// never quietly resume.
#[derive(Debug, Clone)]
pub struct CancellationToken {
    /// The sender is shared rather than the receiver, so a receiver can be
    /// subscribed on demand and the channel can never be closed while any
    /// clone of the token is still alive.
    inner: Arc<watch::Sender<bool>>,
}

impl CancellationToken {
    /// A fresh, uncancelled token.
    pub fn new() -> Self {
        let (sender, _) = watch::channel(false);
        Self {
            inner: Arc::new(sender),
        }
    }

    /// Cancel every stream holding this token, or a clone of it.
    pub fn cancel(&self) {
        // The previous value is of no interest: cancelling twice is the same
        // as cancelling once, and a caller reacting to barge-in should not have
        // to check whether it got there first.
        let _was_already_cancelled = self.inner.send_replace(true);
    }

    /// Whether cancellation has been requested. Cheap enough to call per chunk.
    pub fn is_cancelled(&self) -> bool {
        *self.inner.borrow()
    }

    /// Resolves as soon as cancellation is requested, and never otherwise.
    ///
    /// This is the half an `AtomicBool` cannot provide: it can be raced against
    /// the next network chunk, so the stream stops on the user's timing rather
    /// than the provider's.
    pub async fn cancelled(&self) {
        let mut receiver = self.inner.subscribe();

        // `subscribe` marks the current value as already seen, so a token that
        // was cancelled before anyone waited on it has to be checked directly —
        // otherwise `changed` would wait for a second cancellation that never
        // comes.
        if *receiver.borrow_and_update() {
            return;
        }

        loop {
            match receiver.changed().await {
                Ok(()) => {
                    if *receiver.borrow_and_update() {
                        return;
                    }
                }
                // Unreachable while `self` is alive, since `self` owns the
                // sender. If it ever happened, nobody could cancel and nobody
                // could report progress, so reporting cancellation is the
                // fail-safe answer: it stops generating instead of hanging.
                Err(_sender_dropped) => return,
            }
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_fresh_token_is_not_cancelled_and_never_resolves() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());

        assert!(
            tokio::time::timeout(Duration::from_millis(50), token.cancelled())
                .await
                .is_err(),
            "waiting on an uncancelled token must not resolve"
        );
    }

    #[tokio::test]
    async fn cancelling_is_observed_by_every_clone() {
        let token = CancellationToken::new();
        let clone = token.clone();

        let waiting = tokio::spawn(async move { clone.cancelled().await });

        // Give the task a chance to park on the channel, so this exercises the
        // wake-up path rather than the already-cancelled shortcut.
        tokio::time::sleep(Duration::from_millis(10)).await;
        token.cancel();

        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .expect("a cancelled token must wake its waiters")
            .expect("the waiting task must not panic");
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn waiting_on_an_already_cancelled_token_returns_at_once() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancel();

        assert!(token.is_cancelled());
        tokio::time::timeout(Duration::from_millis(100), token.cancelled())
            .await
            .expect("a token cancelled before anyone waited must resolve immediately");
    }
}
