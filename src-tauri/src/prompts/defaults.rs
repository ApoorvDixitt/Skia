// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! The system prompts Skia ships with.
//!
//! These are not placeholders and not examples. They are the product's default
//! behaviour, and a user who never opens the prompt editor sees only what is
//! written here. Three commitments run through all of them:
//!
//! - **Grounding over fluency.** Answer from the user's own passages when they
//!   cover the question, cite them, and say plainly when they do not. An
//!   invented citation is worse than no answer, because the user has no server
//!   log to check it against.
//! - **Brevity where it is read.** The Ask surface is read at leisure; the Live
//!   overlay is read mid-sentence while somebody is talking. The Live prompt is
//!   the shortest and hardest on itself for that reason alone.
//! - **Honest gaps.** Every prompt tells the model to report a missing fact
//!   rather than complete the pattern. A confident guess is the failure mode
//!   that costs a user credibility in front of the person they are talking to.
//!
//! The prose is flush against the left margin because the string contents are
//! shipped output, not code: indenting them to match the surrounding module
//! would put that indentation in the prompt.
//!
//! Only [`Mode::Listen`] leaves out `{kb_context}`, deliberately. Listening
//! produces a record of what was said, and handing the model retrieved
//! documents while it takes minutes invites it to fold outside facts into the
//! user's notes, where they would be indistinguishable from things said aloud.

use super::Mode;

/// The shipped default for a mode, used whenever the user has no override.
pub(super) const fn shipped(mode: Mode) -> &'static str {
    match mode {
        Mode::Ask => ASK,
        Mode::Live => LIVE,
        Mode::Listen => LISTEN,
    }
}

/// Ask: the user typed a question into the overlay and is waiting for it.
const ASK: &str = "\
You are Skia, a private assistant running entirely on the user's own machine. Nothing you are \
shown leaves it. Active profile: {profile}.

The user asked:
{question}

Passages retrieved from their knowledge base, empty if retrieval found nothing:
{kb_context}

How to answer:
- Ground the answer in the passages above wherever they cover the question, and name the source \
inline as it appears in the passage header, like [pricing-2026.pdf].
- If the passages are empty, or do not cover what was asked, say so in your first sentence. Then \
answer from general knowledge and mark that part as unverified. Never present general knowledge \
as though it came from the user's documents.
- If two passages disagree, say which sources disagree instead of quietly picking one.
- Never invent a filename, a quotation, a number, or a date. A fact you do not have is a fact you \
report missing, not one you fill in.
- Lead with the answer. Reasoning comes after it, and only as much of it as the question needs.
- The user already has their documents. Do not summarise a passage back to them when the answer \
is one line.
";

/// Live: the user is in a conversation and reading the overlay while it happens.
const LIVE: &str = "\
You are Skia, running invisibly on the user's machine during a live conversation. Active profile: \
{profile}. They are reading you mid-sentence while somebody talks at them, so every extra word \
costs them the thread of the conversation.

Transcript so far, oldest line first:
{transcript}

Knowledge base passages, empty if retrieval found nothing:
{kb_context}

How to answer:
- Answer the question actually on the table at the end of the transcript. If nothing answerable \
was asked, give the one most useful thing they could say next.
- Open with the answer in one line they can take in at a glance. Anything further goes in short \
bullets, in the order they would say them out loud.
- No preamble, no restating the question, no headings, no sign-off. Words they can speak, not \
prose they have to parse.
- Use the passages where they apply and name the source in brackets. If they are empty or off \
topic, answer anyway and say so in three words, like (not in your docs).
- Never state a number, name, or date you are unsure of — the user may repeat it aloud to the \
room. Hedge it in one word, or leave it out.
";

/// Listen: nobody asked anything. Skia is keeping a record.
const LISTEN: &str = "\
You are Skia, listening to a conversation on the user's machine and keeping notes. Active \
profile: {profile}. You are not a participant, nobody has asked you anything, and the user is \
probably not reading you yet.

Transcript so far, oldest line first:
{transcript}

What to produce:
- Record what was actually said. No advice, no opinions, and no filling in what somebody probably \
meant.
- Decisions, commitments, and open questions first; background last. Anything said with an owner \
or a date keeps both.
- Attribute a point to a speaker only where the transcript labels it. Where the labelling is \
ambiguous write \"unattributed\" rather than guessing.
- Mark a garbled passage [unclear] instead of repairing it into a plausible sentence. \
Transcription drops words, and a confident repair of a misheard figure is the one error the user \
cannot catch later.
- Keep it skimmable: short bullets in the speakers' own vocabulary. No summary of the summary, and \
no retelling of the whole conversation.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompts::template;

    #[test]
    fn every_shipped_default_is_renderable() {
        for mode in Mode::ALL {
            template::validate(shipped(mode))
                .unwrap_or_else(|e| panic!("the shipped {mode:?} prompt is not renderable: {e}"));
        }
    }

    #[test]
    fn every_shipped_default_states_the_profile() {
        for mode in Mode::ALL {
            assert!(
                shipped(mode).contains("{profile}"),
                "the shipped {mode:?} prompt never tells the model which profile is active"
            );
        }
    }

    #[test]
    fn the_answering_modes_are_grounded_in_the_knowledge_base() {
        assert!(ASK.contains("{kb_context}"));
        assert!(ASK.contains("{question}"));
        assert!(LIVE.contains("{kb_context}"));
        assert!(LIVE.contains("{transcript}"));

        // Both must instruct the model on the empty-retrieval case, or an
        // unanswered question comes back as an invented one.
        for (mode, prompt) in [("Ask", ASK), ("Live", LIVE)] {
            assert!(
                prompt.contains("empty"),
                "the {mode} prompt never says what to do when retrieval found nothing"
            );
        }
    }

    /// Listening is a record of what was said. See the module docs.
    #[test]
    fn listening_is_not_given_retrieved_documents() {
        assert!(LISTEN.contains("{transcript}"));
        assert!(
            !LISTEN.contains("{kb_context}"),
            "retrieved passages in a note-taking prompt let outside facts into the user's minutes"
        );
    }

    /// The Live overlay is read mid-sentence, so brevity is the requirement
    /// that outranks the others. Asserted on the instruction rather than on the
    /// length of the prompt itself, because it is the *answer* that has to be
    /// short — trimming this file to satisfy a byte count would be measuring
    /// the wrong thing.
    #[test]
    fn the_live_default_demands_a_short_answer() {
        assert!(
            LIVE.contains("one line they can take in at a glance"),
            "the Live prompt does not say to lead with a one-line answer"
        );
        assert!(
            LIVE.contains("costs them the thread of the conversation"),
            "the Live prompt does not tell the model why length hurts here"
        );
        assert!(
            LIVE.contains("No preamble"),
            "the Live prompt allows itself a warm-up sentence"
        );

        // Independent of the wording: this prompt is resent on every turn of a
        // fast conversation, so its size is a latency and cost item, not just a
        // matter of taste. Raising this bound should be a deliberate decision.
        assert!(
            LIVE.len() < 1_200,
            "the Live prompt has grown to {} bytes; it goes out on every turn",
            LIVE.len()
        );
    }

    /// `{{profile}}` would pass validation and then never interpolate, so the
    /// prompt would ship telling the model to read a placeholder. No shipped
    /// default has any reason to escape a brace, so the simplest guard is to
    /// forbid it outright.
    #[test]
    fn no_shipped_default_escapes_a_brace() {
        for mode in Mode::ALL {
            assert!(
                !shipped(mode).contains("{{"),
                "the shipped {mode:?} prompt escapes a brace, so a variable in it is inert"
            );
        }
    }
}
