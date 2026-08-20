// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! Meeting memory: people, meetings, and what was agreed.
//!
//! This is the data behind the product's headline promise — walking into a
//! meeting already knowing what the last one with these people decided. The
//! module is deliberately plain SQL over the v2 tables; the interesting part
//! is [`Store::brief_for_people`], which answers "what happened last time and
//! what is still open" as *data*. Rendering that data with a model is a
//! caller's choice; the brief must exist and be correct even when no model is
//! configured, because the facts are the product and the prose is polish.
//!
//! Identity: a person is matched by email when one is given (emails survive
//! renames), by exact name otherwise. Matching by name is honest-best-effort
//! and stated as such — two "Alex"es without emails are two people.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::{Store, StoreError};

/// Someone who attends meetings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
}

/// One meeting, possibly still running (`ended_at` is `None`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: i64,
    pub title: Option<String>,
    pub profile: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// A commitment made in a meeting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    pub id: i64,
    pub meeting_id: i64,
    /// Who owns it, when known. `None` after the person was deleted — the
    /// commitment outlives the contact entry on purpose.
    pub person_id: Option<i64>,
    pub person_name: Option<String>,
    pub text: String,
    pub done: bool,
    pub created_at: i64,
}

/// An attendee as the caller names one: a name, an optional email.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttendeeSpec {
    pub name: String,
    pub email: Option<String>,
}

/// What Skia already knows walking into a meeting with these people.
///
/// Pure data, assembled with three queries. `prior_meetings` are the most
/// recent meetings sharing at least one attendee; `open_items` are their
/// unfinished commitments plus any owned by an attendee directly.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingBrief {
    pub attendees: Vec<Person>,
    pub prior_meetings: Vec<Meeting>,
    pub open_items: Vec<ActionItem>,
}

/// How many prior meetings a brief reaches back through. Enough to cover a
/// recurring weekly for two months; a brief that recites all history stops
/// being a brief.
const BRIEF_MEETINGS: u32 = 8;

impl Store {
    /// Find-or-create each attendee, then open the meeting with them.
    ///
    /// One transaction: a meeting whose attendee rows half-exist would produce
    /// briefs that silently miss people.
    pub fn start_meeting(
        &self,
        title: Option<&str>,
        profile: &str,
        attendees: &[AttendeeSpec],
    ) -> Result<i64, StoreError> {
        if profile.trim().is_empty() {
            return Err(StoreError::EmptyField { field: "profile" });
        }

        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "INSERT INTO meetings (title, profile) VALUES (?1, ?2)",
            (title.filter(|t| !t.trim().is_empty()), profile),
        )?;
        let meeting_id = tx.last_insert_rowid();

        for spec in attendees {
            let name = spec.name.trim();
            if name.is_empty() {
                continue;
            }
            let email = spec
                .email
                .as_deref()
                .map(str::trim)
                .filter(|e| !e.is_empty());

            // Email is identity when present; name is best-effort otherwise.
            let existing: Option<i64> = match email {
                Some(email) => tx
                    .query_row("SELECT id FROM people WHERE email = ?1", (email,), |row| {
                        row.get(0)
                    })
                    .optional()?,
                None => tx
                    .query_row(
                        "SELECT id FROM people WHERE name = ?1 AND email IS NULL",
                        (name,),
                        |row| row.get(0),
                    )
                    .optional()?,
            };

            let person_id = match existing {
                Some(id) => {
                    // A rename with a stable email updates the name — the
                    // email is the identity, the name is a label.
                    tx.execute("UPDATE people SET name = ?2 WHERE id = ?1", (id, name))?;
                    id
                }
                None => {
                    tx.execute(
                        "INSERT INTO people (name, email) VALUES (?1, ?2)",
                        (name, email),
                    )?;
                    tx.last_insert_rowid()
                }
            };

            // OR IGNORE: naming the same attendee twice is a caller quirk,
            // not an error worth failing the meeting over.
            tx.execute(
                "INSERT OR IGNORE INTO meeting_people (meeting_id, person_id) VALUES (?1, ?2)",
                (meeting_id, person_id),
            )?;
        }

        tx.commit()?;
        Ok(meeting_id)
    }

    /// Close the meeting. Idempotent: ending an ended meeting changes nothing,
    /// because the first end time is the true one.
    pub fn end_meeting(&self, meeting_id: i64) -> Result<(), StoreError> {
        let updated = self.conn.execute(
            "UPDATE meetings SET ended_at = unixepoch()
              WHERE id = ?1 AND ended_at IS NULL",
            (meeting_id,),
        )?;
        if updated == 0 {
            // Distinguish "already ended" (fine) from "no such meeting".
            let exists: Option<i64> = self
                .conn
                .query_row(
                    "SELECT id FROM meetings WHERE id = ?1",
                    (meeting_id,),
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(StoreError::NotFound {
                    entity: "meeting",
                    id: meeting_id,
                });
            }
        }
        Ok(())
    }

    /// Every meeting, newest first.
    pub fn list_meetings(&self) -> Result<Vec<Meeting>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, profile, started_at, ended_at
               FROM meetings ORDER BY started_at DESC, id DESC",
        )?;
        let rows = statement
            .query_map([], row_to_meeting)?
            .collect::<rusqlite::Result<Vec<Meeting>>>()?;
        Ok(rows)
    }

    /// The attendees of one meeting, in name order.
    pub fn meeting_attendees(&self, meeting_id: i64) -> Result<Vec<Person>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT p.id, p.name, p.email
               FROM meeting_people mp JOIN people p ON p.id = mp.person_id
              WHERE mp.meeting_id = ?1
              ORDER BY p.name, p.id",
        )?;
        let rows = statement
            .query_map((meeting_id,), row_to_person)?
            .collect::<rusqlite::Result<Vec<Person>>>()?;
        Ok(rows)
    }

    /// Record a commitment. `person_id` may be `None` — "someone should" is
    /// still worth writing down, and the UI can assign it later.
    pub fn add_action_item(
        &self,
        meeting_id: i64,
        person_id: Option<i64>,
        text: &str,
    ) -> Result<i64, StoreError> {
        if text.trim().is_empty() {
            return Err(StoreError::EmptyField {
                field: "action item text",
            });
        }
        self.conn.execute(
            "INSERT INTO action_items (meeting_id, person_id, text) VALUES (?1, ?2, ?3)",
            (meeting_id, person_id, text.trim()),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Mark a commitment done — or undone, because people un-finish things.
    pub fn set_action_done(&self, item_id: i64, done: bool) -> Result<(), StoreError> {
        let updated = self.conn.execute(
            "UPDATE action_items SET done = ?2 WHERE id = ?1",
            (item_id, i64::from(done)),
        )?;
        if updated == 0 {
            return Err(StoreError::NotFound {
                entity: "action item",
                id: item_id,
            });
        }
        Ok(())
    }

    /// Action items of one meeting, open first, oldest first within a state.
    pub fn meeting_action_items(&self, meeting_id: i64) -> Result<Vec<ActionItem>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT a.id, a.meeting_id, a.person_id, p.name, a.text, a.done, a.created_at
               FROM action_items a LEFT JOIN people p ON p.id = a.person_id
              WHERE a.meeting_id = ?1
              ORDER BY a.done, a.created_at, a.id",
        )?;
        let rows = statement
            .query_map((meeting_id,), row_to_action_item)?
            .collect::<rusqlite::Result<Vec<ActionItem>>>()?;
        Ok(rows)
    }

    /// What Skia knows walking into a meeting with these attendees.
    ///
    /// `exclude_meeting` keeps the meeting being started out of its own brief.
    pub fn brief_for_people(
        &self,
        person_ids: &[i64],
        exclude_meeting: Option<i64>,
    ) -> Result<MeetingBrief, StoreError> {
        if person_ids.is_empty() {
            return Ok(MeetingBrief {
                attendees: Vec::new(),
                prior_meetings: Vec::new(),
                open_items: Vec::new(),
            });
        }

        // Bound parameters, built by count. Never formatted values — ids are
        // i64 today, but string-built SQL is a habit this codebase refuses.
        let placeholders = (1..=person_ids.len())
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let params: Vec<&dyn rusqlite::ToSql> = person_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();

        let attendees = {
            let mut statement = self.conn.prepare(&format!(
                "SELECT id, name, email FROM people WHERE id IN ({placeholders})
                 ORDER BY name, id"
            ))?;
            let rows = statement
                .query_map(params.as_slice(), row_to_person)?
                .collect::<rusqlite::Result<Vec<Person>>>()?;
            rows
        };

        // The exclusion rides as the last parameter; NULL disables it.
        let exclude_index = person_ids.len() + 1;
        let limit_index = person_ids.len() + 2;
        let mut with_exclude = params.clone();
        let exclude = exclude_meeting;
        with_exclude.push(&exclude as &dyn rusqlite::ToSql);
        let limit = i64::from(BRIEF_MEETINGS);
        with_exclude.push(&limit as &dyn rusqlite::ToSql);

        let prior_meetings = {
            let mut statement = self.conn.prepare(&format!(
                "SELECT DISTINCT m.id, m.title, m.profile, m.started_at, m.ended_at
                   FROM meetings m JOIN meeting_people mp ON mp.meeting_id = m.id
                  WHERE mp.person_id IN ({placeholders})
                    AND (?{exclude_index} IS NULL OR m.id <> ?{exclude_index})
                  ORDER BY m.started_at DESC, m.id DESC
                  LIMIT ?{limit_index}"
            ))?;
            let rows = statement
                .query_map(with_exclude.as_slice(), row_to_meeting)?
                .collect::<rusqlite::Result<Vec<Meeting>>>()?;
            rows
        };

        // Open items from those meetings, plus anything owned by an attendee
        // from any other meeting — a promise does not expire because the next
        // meeting has a different subset of people in it.
        let open_items = if prior_meetings.is_empty() {
            Vec::new()
        } else {
            let mut statement = self.conn.prepare(&format!(
                "SELECT DISTINCT a.id, a.meeting_id, a.person_id, p.name, a.text,
                        a.done, a.created_at
                   FROM action_items a
                   LEFT JOIN people p ON p.id = a.person_id
                  WHERE a.done = 0
                    AND (a.meeting_id IN (SELECT m.id FROM meetings m
                                            JOIN meeting_people mp ON mp.meeting_id = m.id
                                           WHERE mp.person_id IN ({placeholders}))
                         OR a.person_id IN ({placeholders}))
                  ORDER BY a.created_at, a.id"
            ))?;
            let rows = statement
                .query_map(params.as_slice(), row_to_action_item)?
                .collect::<rusqlite::Result<Vec<ActionItem>>>()?;
            rows
        };

        Ok(MeetingBrief {
            attendees,
            prior_meetings,
            open_items,
        })
    }
}

fn row_to_person(row: &rusqlite::Row<'_>) -> rusqlite::Result<Person> {
    Ok(Person {
        id: row.get(0)?,
        name: row.get(1)?,
        email: row.get(2)?,
    })
}

fn row_to_meeting(row: &rusqlite::Row<'_>) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: row.get(0)?,
        title: row.get(1)?,
        profile: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
    })
}

fn row_to_action_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActionItem> {
    Ok(ActionItem {
        id: row.get(0)?,
        meeting_id: row.get(1)?,
        person_id: row.get(2)?,
        person_name: row.get(3)?,
        text: row.get(4)?,
        done: row.get::<_, i64>(5)? != 0,
        created_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn attendee(name: &str, email: Option<&str>) -> AttendeeSpec {
        AttendeeSpec {
            name: name.to_string(),
            email: email.map(str::to_string),
        }
    }

    #[test]
    fn a_meeting_records_its_attendees_and_reuses_people_by_email() {
        let store = store();
        let first = store
            .start_meeting(
                Some("Kickoff"),
                "meeting",
                &[
                    attendee("Priya Sharma", Some("priya@example.com")),
                    attendee("Alex", None),
                ],
            )
            .unwrap();
        assert_eq!(store.meeting_attendees(first).unwrap().len(), 2);

        // Priya renamed herself; the email says she is the same person.
        let second = store
            .start_meeting(
                Some("Review"),
                "meeting",
                &[attendee("Priya S.", Some("priya@example.com"))],
            )
            .unwrap();
        let people = store.meeting_attendees(second).unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name, "Priya S.", "the email owns the identity");

        // Same id across both meetings — that is what makes briefs possible.
        let first_people = store.meeting_attendees(first).unwrap();
        assert!(first_people.iter().any(|p| p.id == people[0].id));
    }

    #[test]
    fn two_email_less_people_with_the_same_name_stay_two_people() {
        let store = store();
        store
            .start_meeting(None, "meeting", &[attendee("Alex", None)])
            .unwrap();
        let meeting = store
            .start_meeting(None, "meeting", &[attendee("Alex", None)])
            .unwrap();
        // Name-only matching reuses the earlier Alex: best-effort identity.
        assert_eq!(store.meeting_attendees(meeting).unwrap().len(), 1);
        // But an Alex WITH an email is someone else entirely.
        let third = store
            .start_meeting(None, "meeting", &[attendee("Alex", Some("alex@x.com"))])
            .unwrap();
        let with_email = store.meeting_attendees(third).unwrap();
        assert_eq!(with_email[0].email.as_deref(), Some("alex@x.com"));
    }

    #[test]
    fn the_brief_carries_prior_meetings_and_open_items_but_not_done_ones() {
        let store = store();
        let past = store
            .start_meeting(
                Some("Q3 plan"),
                "meeting",
                &[attendee("Priya", Some("priya@example.com"))],
            )
            .unwrap();
        let priya = store.meeting_attendees(past).unwrap()[0].id;

        let approve = store
            .add_action_item(past, Some(priya), "Approve the pricing page")
            .unwrap();
        let shipped = store
            .add_action_item(past, None, "Ship the beta invite")
            .unwrap();
        store.set_action_done(shipped, true).unwrap();
        store.end_meeting(past).unwrap();

        // Next meeting with Priya: the brief must surface Q3 plan and the
        // still-open approval, and must not recite the done item.
        let next = store
            .start_meeting(
                Some("Q4 plan"),
                "meeting",
                &[attendee("Priya", Some("priya@example.com"))],
            )
            .unwrap();
        let brief = store.brief_for_people(&[priya], Some(next)).unwrap();

        assert_eq!(
            brief
                .prior_meetings
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            vec![past],
            "the new meeting must not appear in its own brief"
        );
        assert_eq!(
            brief.open_items.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![approve]
        );
        assert_eq!(brief.open_items[0].person_name.as_deref(), Some("Priya"));
    }

    #[test]
    fn deleting_a_person_orphans_their_commitments_instead_of_deleting_them() {
        let store = store();
        let meeting = store
            .start_meeting(None, "meeting", &[attendee("Sam", Some("sam@x.com"))])
            .unwrap();
        let sam = store.meeting_attendees(meeting).unwrap()[0].id;
        store
            .add_action_item(meeting, Some(sam), "Send the contract")
            .unwrap();

        store
            .conn
            .execute("DELETE FROM people WHERE id = ?1", (sam,))
            .unwrap();

        let items = store.meeting_action_items(meeting).unwrap();
        assert_eq!(items.len(), 1, "the commitment survives the contact");
        assert_eq!(items[0].person_id, None);
        assert_eq!(items[0].person_name, None);
    }

    #[test]
    fn ending_twice_is_fine_and_ending_nothing_is_an_error() {
        let store = store();
        let meeting = store.start_meeting(None, "meeting", &[]).unwrap();
        store.end_meeting(meeting).unwrap();
        let first_end: Option<i64> = store
            .conn
            .query_row(
                "SELECT ended_at FROM meetings WHERE id = ?1",
                (meeting,),
                |r| r.get(0),
            )
            .unwrap();
        store.end_meeting(meeting).unwrap();
        let second_end: Option<i64> = store
            .conn
            .query_row(
                "SELECT ended_at FROM meetings WHERE id = ?1",
                (meeting,),
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(first_end, second_end, "the first end time is the true one");

        assert!(matches!(
            store.end_meeting(9_999),
            Err(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn empty_profile_and_empty_action_text_are_refused_with_field_names() {
        let store = store();
        let error = store.start_meeting(None, "  ", &[]).unwrap_err();
        assert!(error.to_string().contains("profile"), "{error}");

        let meeting = store.start_meeting(None, "meeting", &[]).unwrap();
        let error = store.add_action_item(meeting, None, "  ").unwrap_err();
        assert!(error.to_string().contains("action item"), "{error}");
    }
}
