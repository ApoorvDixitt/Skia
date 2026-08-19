// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The cheap decision about whether a turn is worth a knowledge-base lookup.
//!
//! Retrieval is fired speculatively against partial transcripts, so this runs
//! on every endpointed utterance in a live meeting. It has to be free: no
//! model, no allocation-heavy parsing, no network. It is a handful of string
//! rules, in this order:
//!
//! 1. Nothing indexable in the turn at all — no lookup.
//! 2. Not one word in it carries content (`"hey, good morning everyone"`,
//!    `"what's up?"`) — no lookup, even if it ends in a question mark. Two
//!    reasons, and the second is the stronger one: `"how are you?"` is not a
//!    question about the user's documents, and a query made entirely of
//!    function words has nothing to search *for* — BM25 would rank on
//!    stopwords.
//! 3. It ends in a question mark — look it up.
//! 4. It opens with an interrogative or a "go and find out" verb
//!    (`what`, `why`, `does`, `compare`, `summarise`, …) — look it up. This is
//!    what catches dictated questions, since transcripts rarely carry final
//!    punctuation.
//! 5. It is at least [`SUBSTANTIVE_WORDS`] words long — look it up.
//! 6. Otherwise — no lookup.
//!
//! **The bias is deliberate.** A false positive costs one BM25 query against a
//! local index, off the critical path; a false negative costs a confident
//! answer with no grounding in the documents the user actually gave us. So
//! rules 3–5 are generous and only the explicit small-talk rule takes a turn
//! out of contention.
//!
//! This is not machine learning and is not meant to become machine learning.
//! It is a filter to stop `"sounds good, thanks"` hitting the index.

/// Words that name nothing to look up: greetings, thanks, back-channel noise,
/// and the function words that hold them together. A turn made *entirely* of
/// these gets no lookup.
///
/// Interrogatives are in here too (`what`, `how`), which looks wrong until the
/// rule is read carefully: a turn is only suppressed when *every* word is on
/// this list, so `"what's up?"` is out and `"what's our refund policy?"` is very
/// much in. It is the absence of any content word that decides, not the presence
/// of `what`.
///
/// Only closed-class and social words belong here. Adding a content word (say
/// `"pricing"`) would silently suppress lookups for real questions, which is
/// the failure this whole module is meant to avoid.
// Packed by hand: one word per line is eighty lines of noise, and this list is
// meant to be read as vocabulary.
#[rustfmt::skip]
const CONTENTLESS: &[&str] = &[
    "a", "afternoon", "all", "alright", "am", "and", "are", "awesome", "bye", "cheers", "cool",
    "do", "doing", "evening", "everybody", "everyone", "fine", "folks", "for", "good",
    "goodbye", "great", "guys", "have", "hello", "hey", "hi", "how", "hows", "i", "im", "is",
    "it", "later", "lot", "me", "morning", "much", "my", "new", "nice", "no", "nope", "np",
    "ok", "okay", "perfect", "pleasure", "s", "see", "so", "sorry", "sounds", "sure", "team",
    "thank", "thanks", "thats", "the", "there", "thx", "to", "today", "tomorrow", "too", "uh",
    "um", "up", "very", "welcome", "well", "what", "whats", "with", "yeah", "yep", "yes",
    "you", "your", "yourself",
];

/// Words that open a request for information. A turn starting with one of these
/// is treated as a question even without a question mark, which is the normal
/// case for speech.
// Packed for the same reason as the list above.
#[rustfmt::skip]
const LOOKUP_OPENERS: &[&str] = &[
    "am", "are", "can", "clarify", "compare", "confirm", "contrast", "could", "define",
    "describe", "did", "do", "does", "explain", "find", "give", "has", "have", "how", "is",
    "list", "look", "outline", "recap", "remind", "should", "show", "summarise", "summarize",
    "tell", "walk", "was", "were", "what", "whats", "when", "where", "which", "who", "whom",
    "whose", "why", "will", "would",
];

/// A turn at least this long is assumed to carry a claim or a request worth
/// grounding, even if nothing else in it looks like a question.
pub const SUBSTANTIVE_WORDS: usize = 5;

/// Whether `turn` warrants a knowledge-base lookup.
///
/// See the module documentation for the rules and for why this errs towards
/// `true`.
pub fn needs_retrieval(turn: &str) -> bool {
    let words = words_of(turn);
    if words.is_empty() {
        return false;
    }

    // Rule 2 comes before the question rules on purpose: "how are you?" is a
    // question in form only.
    if words
        .iter()
        .all(|word| CONTENTLESS.contains(&word.as_str()))
    {
        return false;
    }

    // A question mark is the strongest signal there is, when it survives at all
    // — transcription often drops it, which is what rule 4 is for.
    if turn.trim_end().ends_with(['?', '？']) {
        return true;
    }

    if words
        .first()
        .is_some_and(|first| LOOKUP_OPENERS.contains(&first.as_str()))
    {
        return true;
    }

    words.len() >= SUBSTANTIVE_WORDS
}

/// The comparable words of a turn: lowercased, with punctuation and any other
/// non-alphanumeric character treated as a separator.
///
/// Splitting on `is_alphanumeric` rather than on whitespace is what makes
/// `"What's"` two words (`what`, `s`) and `"thanks!"` one (`thanks`), so the
/// lists above never need to carry punctuation variants.
fn words_of(turn: &str) -> Vec<String> {
    turn.split(|character: char| !char::is_alphanumeric(character))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greetings_and_back_channel_need_no_lookup() {
        for turn in [
            "Hi",
            "hello!",
            "Hey, good morning!",
            "Good morning everyone.",
            "How are you?",
            "how are you doing today",
            "Thanks!",
            "thanks a lot",
            "Thank you so much",
            "ok",
            "Okay, sounds good.",
            "yep",
            "See you tomorrow!",
            "what's up?",
        ] {
            assert!(
                !needs_retrieval(turn),
                "{turn:?} should not trigger a lookup"
            );
        }
    }

    #[test]
    fn substantive_questions_need_a_lookup() {
        for turn in [
            "What is our refund policy for annual plans?",
            "whats the refund window",
            "Refund policy?",
            "Why did we move off Deepgram",
            "Explain how the audio device hot-swap works",
            "Summarise the data handling section",
            "Compare the two pricing tiers for me",
            "Does the overlay show up in a Zoom screen share",
            "The customer is asking about SOC 2 and I have no idea",
            "remind me what we promised about on-device processing",
        ] {
            assert!(needs_retrieval(turn), "{turn:?} should trigger a lookup");
        }
    }

    #[test]
    fn a_turn_with_nothing_in_it_needs_no_lookup() {
        for turn in ["", "   ", "\n\t", "...", "???", "—", "😀"] {
            assert!(!needs_retrieval(turn), "{turn:?} has nothing to look up");
        }
    }

    #[test]
    fn one_content_word_is_enough_to_earn_a_lookup() {
        // Every word but one is on the contentless list; the one that is not
        // carries the question, so suppressing these would be the expensive
        // mistake.
        assert!(needs_retrieval("so what is the SLA"));
        assert!(needs_retrieval("thanks, and the pricing?"));
        assert!(needs_retrieval("what's our refund policy?"));
        assert!(
            !needs_retrieval("thanks, and you?"),
            "no content word means no lookup, question mark or not"
        );
        assert!(
            !needs_retrieval("what's new?"),
            "an interrogative with nothing after it has nothing to search for"
        );
    }

    #[test]
    fn the_gate_errs_towards_looking_things_up() {
        // Neither of these needs the knowledge base, and both get a lookup: any
        // sentence of five words that is not pure pleasantry passes. That is the
        // trade named in the module comment — a wasted local query is cheaper
        // than an ungrounded answer — and it is asserted here so that a future
        // change to the rules is a deliberate one.
        assert!(needs_retrieval("Sorry, my nephew is here."));
        assert!(needs_retrieval("Let us get started with the agenda."));
    }

    #[test]
    fn short_statements_stay_below_the_threshold() {
        assert!(!needs_retrieval("that sounds fine"));
        assert!(!needs_retrieval("Perfect, thanks."));
        assert_eq!(
            SUBSTANTIVE_WORDS, 5,
            "the threshold is documented in the module comment; changing it \
             changes how chatty a meeting can get before Skia starts looking \
             things up"
        );
    }

    #[test]
    fn words_are_lowercased_and_split_on_punctuation() {
        assert_eq!(words_of("What's THAT?"), vec!["what", "s", "that"]);
        assert_eq!(words_of("bge-m3"), vec!["bge", "m3"]);
        assert_eq!(words_of("   "), Vec::<String>::new());
        assert_eq!(
            words_of("Café"),
            vec!["café"],
            "non-ASCII letters are words"
        );
    }
}
