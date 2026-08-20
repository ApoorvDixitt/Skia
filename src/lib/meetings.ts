// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import {
  asBoolean,
  asInteger,
  asNullableString,
  asRecord,
  asString,
  mapArray,
} from "./ipc";

/**
 * The meeting-memory contract, mirrored from Rust. Checked, never cast, like
 * every IPC surface — a malformed brief must fail loudly, not render as an
 * empty one that reads like "nothing was agreed last time".
 */

export interface Person {
  id: number;
  name: string;
  email: string | null;
}

export interface Meeting {
  id: number;
  title: string | null;
  profile: string;
  startedAt: number;
  /** `null` while the meeting is still running. */
  endedAt: number | null;
}

export interface ActionItem {
  id: number;
  meetingId: number;
  personId: number | null;
  personName: string | null;
  text: string;
  done: boolean;
  createdAt: number;
}

/** What Skia already knows walking into a meeting with these people. */
export interface MeetingBrief {
  attendees: Person[];
  priorMeetings: Meeting[];
  openItems: ActionItem[];
}

export interface MeetingStarted {
  meetingId: number;
  brief: MeetingBrief;
}

export interface MeetingDetail {
  meeting: Meeting;
  attendees: Person[];
  actionItems: ActionItem[];
}

export interface AttendeeSpec {
  name: string;
  email: string | null;
}

function parsePerson(value: unknown, at: string): Person {
  const row = asRecord(value, at);
  return {
    id: asInteger(row, at, "id"),
    name: asString(row, at, "name"),
    email: asNullableString(row, at, "email"),
  };
}

function parseMeeting(value: unknown, at: string): Meeting {
  const row = asRecord(value, at);
  return {
    id: asInteger(row, at, "id"),
    title: asNullableString(row, at, "title"),
    profile: asString(row, at, "profile"),
    startedAt: asInteger(row, at, "startedAt"),
    endedAt: row["endedAt"] == null ? null : asInteger(row, at, "endedAt"),
  };
}

function parseActionItem(value: unknown, at: string): ActionItem {
  const row = asRecord(value, at);
  return {
    id: asInteger(row, at, "id"),
    meetingId: asInteger(row, at, "meetingId"),
    personId: row["personId"] == null ? null : asInteger(row, at, "personId"),
    personName: asNullableString(row, at, "personName"),
    text: asString(row, at, "text"),
    done: asBoolean(row, at, "done"),
    createdAt: asInteger(row, at, "createdAt"),
  };
}

function parseBrief(value: unknown, at: string): MeetingBrief {
  const row = asRecord(value, at);
  return {
    attendees: mapArray(row["attendees"], `${at}.attendees`, parsePerson),
    priorMeetings: mapArray(
      row["priorMeetings"],
      `${at}.priorMeetings`,
      parseMeeting,
    ),
    openItems: mapArray(row["openItems"], `${at}.openItems`, parseActionItem),
  };
}

export async function startMeeting(
  title: string | null,
  profile: string,
  attendees: AttendeeSpec[],
): Promise<MeetingStarted> {
  const raw = await invoke<unknown>("meeting_start", {
    title,
    profile,
    attendees,
  });
  const at = "meeting_start";
  const row = asRecord(raw, at);
  return {
    meetingId: asInteger(row, at, "meetingId"),
    brief: parseBrief(row["brief"], `${at}.brief`),
  };
}

export async function endMeeting(meetingId: number): Promise<void> {
  await invoke("meeting_end", { meetingId });
}

export async function listMeetings(): Promise<Meeting[]> {
  const raw = await invoke<unknown>("meetings_list");
  return mapArray(raw, "meetings_list", parseMeeting);
}

export async function meetingDetail(
  meetingId: number,
): Promise<MeetingDetail> {
  const raw = await invoke<unknown>("meeting_detail", { meetingId });
  const at = "meeting_detail";
  const row = asRecord(raw, at);
  return {
    meeting: parseMeeting(row["meeting"], `${at}.meeting`),
    attendees: mapArray(row["attendees"], `${at}.attendees`, parsePerson),
    actionItems: mapArray(
      row["actionItems"],
      `${at}.actionItems`,
      parseActionItem,
    ),
  };
}

export async function addActionItem(
  meetingId: number,
  personId: number | null,
  text: string,
): Promise<number> {
  const raw = await invoke<unknown>("meeting_add_action", {
    meetingId,
    personId,
    text,
  });
  if (typeof raw !== "number" || !Number.isInteger(raw)) {
    throw new Error("meeting_add_action should return the new item's id.");
  }
  return raw;
}

export async function setActionDone(
  itemId: number,
  done: boolean,
): Promise<void> {
  await invoke("meeting_set_action_done", { itemId, done });
}

export async function appendNote(
  meetingId: number,
  speaker: string | null,
  text: string,
): Promise<void> {
  await invoke("meeting_append_note", { meetingId, speaker, text });
}
