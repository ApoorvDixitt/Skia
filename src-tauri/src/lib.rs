// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

mod catalog;
mod panel;
mod stealth;

pub mod audio;
pub mod prompts;
pub mod providers;
pub mod rag;
pub mod secrets;
pub mod storage;
pub mod sync;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use catalog::{Hosting, CATALOG};
use prompts::{Length, Mode, Profile, PromptBundle, PromptVars, Tone};
use providers::{
    CancellationToken, ChatMessage, ChatRequest, EmbeddingsClient, EmbeddingsConfig, MockProvider,
    OpenAiCompatible, OpenAiConfig, Provider,
};
use rag::KnowledgeBase;
use secrets::SecretStore;
use stealth::StealthStatus;
use storage::{Message, Session, Store};

/// The default overlay hotkey. Silent by design: registering a global shortcut
/// produces no sound, banner, or notification on either platform.
const DEFAULT_TOGGLE_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "cmd+shift+space"
} else {
    "ctrl+shift+space"
};

const KEY_CAPTURE_EXCLUSION: &str = "stealth.capture_exclusion_requested";
/// Which catalog provider computes embeddings, or unset for keyword-only.
const KEY_EMBEDDINGS_PROVIDER: &str = "embeddings.provider_id";
/// The embedding model — also the namespace stored vectors live in, so
/// changing it invalidates the semantic index until re-embedding catches up.
const KEY_EMBEDDINGS_MODEL: &str = "embeddings.model";
/// Chunks embedded per `kb_embed_pending` call. Small enough that one call
/// stays inside a request timeout; the UI loops until nothing remains.
const EMBED_BATCH: u32 = 32;
/// Whether first-run setup has been finished or deliberately skipped. Skipping
/// counts: onboarding must not reappear just because the user declined it.
const KEY_ONBOARDING_DONE: &str = "onboarding.completed";
const KEYCHAIN_SERVICE: &str = "dev.skia.apikeys";
/// The compact always-on-top bar that sits over a call.
const OVERLAY_LABEL: &str = "overlay";
/// The full window for the knowledge base, history, and settings. Kept separate
/// so the overlay can stay small: anything that needs room lives here.
const DASHBOARD_LABEL: &str = "dashboard";
/// How many knowledge-base chunks to put in front of the model.
const RETRIEVAL_LIMIT: u32 = 6;

/// Marker placed in managed state during setup when first-run setup is pending,
/// so `RunEvent::Ready` knows to bring the dashboard up. A marker rather than a
/// flag on `AppState` because it is read once and never changes.
struct NeedsSetup;

/// Shared application state. `rusqlite::Connection` is not `Sync`, so both
/// databases sit behind mutexes.
struct AppState {
    store: Mutex<Store>,
    kb: Mutex<KnowledgeBase>,
    secrets: SecretStore,
    prompts: Mutex<PromptBundle>,
    /// In-flight generations, so barge-in can cancel one mid-stream.
    inflight: Mutex<HashMap<String, CancellationToken>>,
    next_request: AtomicU64,
    /// The audio engine's handle. `Arc` because a probe blocks for its whole
    /// recording and runs on a blocking task that must own its reference.
    audio: std::sync::Arc<audio::Handle>,
}

impl AppState {
    fn with_store<T>(
        &self,
        f: impl FnOnce(&Store) -> Result<T, storage::StoreError>,
    ) -> Result<T, String> {
        let guard = match self.store.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&guard).map_err(|e| e.to_string())
    }

    fn with_kb<T>(
        &self,
        f: impl FnOnce(&KnowledgeBase) -> Result<T, rag::RagError>,
    ) -> Result<T, String> {
        let guard = match self.kb.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&guard).map_err(|e| e.to_string())
    }
}

fn read_capture_preference(store: &Store) -> Result<bool, storage::StoreError> {
    Ok(store
        .get_setting(KEY_CAPTURE_EXCLUSION)?
        .map(|v| v == "true")
        .unwrap_or(true))
}

/// Defaults to `false`, so a fresh install gets setup and an existing one that
/// somehow lost the row gets it again rather than silently skipping it.
fn read_onboarding_done(store: &Store) -> Result<bool, storage::StoreError> {
    Ok(store
        .get_setting(KEY_ONBOARDING_DONE)?
        .map(|v| v == "true")
        .unwrap_or(false))
}

// ------------------------------------------------------------- onboarding ----

#[tauri::command]
fn onboarding_done(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    state.with_store(read_onboarding_done)
}

/// Records completion. Also used with `false` to re-run setup from the dashboard.
#[tauri::command]
fn set_onboarding_done(done: bool, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.with_store(|store| {
        store.set_setting(KEY_ONBOARDING_DONE, if done { "true" } else { "false" })
    })
}

// ---------------------------------------------------------------- stealth ----

#[tauri::command]
fn stealth_status(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<StealthStatus, String> {
    let requested = state.with_store(read_capture_preference)?;
    stealth::status(&window, requested).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_capture_exclusion(
    enabled: bool,
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<StealthStatus, String> {
    state.with_store(|store| {
        store.set_setting(
            KEY_CAPTURE_EXCLUSION,
            if enabled { "true" } else { "false" },
        )
    })?;
    stealth::apply(&window, enabled).map_err(|e| e.to_string())
}

// -------------------------------------------------------------- providers ----

/// What the UI is allowed to know about a provider. Deliberately never carries
/// the key itself — only whether one exists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderInfo {
    id: String,
    label: String,
    configured: bool,
    is_mock: bool,
    is_local: bool,
    needs_api_key: bool,
    model: String,
    note: String,
    api_key_url: Option<String>,
    /// The provider's default embedding model, when it serves `/embeddings`.
    embedding_model: Option<String>,
}

#[tauri::command]
fn providers_list(state: tauri::State<'_, AppState>) -> Result<Vec<ProviderInfo>, String> {
    let mut out = Vec::with_capacity(CATALOG.len());
    for entry in CATALOG {
        // A local or mock provider needs no key, so it is always usable. For a
        // cloud provider, ask the keychain — and let a keychain failure surface
        // rather than reporting "not configured", which would be a different
        // and misleading problem.
        let configured = if entry.needs_api_key() {
            state
                .secrets
                .has_api_key(entry.id)
                .map_err(|e| format!("could not read the keychain for {}: {e}", entry.id))?
        } else {
            true
        };
        out.push(ProviderInfo {
            id: entry.id.to_string(),
            label: entry.label.to_string(),
            configured,
            is_mock: entry.hosting == Hosting::Mock,
            is_local: entry.hosting == Hosting::Local,
            needs_api_key: entry.needs_api_key(),
            model: entry.default_model.to_string(),
            note: entry.note.to_string(),
            api_key_url: entry.api_key_url.map(str::to_string),
            embedding_model: entry.embedding_model.map(str::to_string),
        });
    }
    Ok(out)
}

/// The per-role fallback order: which providers Skia would reach for when it
/// needs a fast live answer, careful reasoning, or vision. Exposed so a settings
/// screen can show routing rather than keeping a second list that drifts.
#[tauri::command]
fn role_defaults() -> HashMap<String, Vec<String>> {
    providers::ProviderRole::ALL
        .iter()
        .map(|role| {
            (
                role.alias().to_string(),
                catalog::defaults_for_role(*role)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            )
        })
        .collect()
}

#[tauri::command]
fn set_api_key(
    provider_id: String,
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let entry =
        catalog::entry(&provider_id).ok_or_else(|| format!("unknown provider: {provider_id}"))?;
    if !entry.needs_api_key() {
        return Err(format!("{} does not take an API key", entry.label));
    }
    state
        .secrets
        .set_api_key(&provider_id, &key)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_api_key(provider_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .secrets
        .delete_api_key(&provider_id)
        .map_err(|e| e.to_string())
}

/// Builds a live provider from the catalog plus the keychain.
fn build_provider(provider_id: &str, secrets: &SecretStore) -> Result<Box<dyn Provider>, String> {
    let entry =
        catalog::entry(provider_id).ok_or_else(|| format!("unknown provider: {provider_id}"))?;

    if entry.hosting == Hosting::Mock {
        return Ok(Box::new(MockProvider::new(entry.id)));
    }

    let mut config = OpenAiConfig::new(entry.id, entry.base_url, entry.default_model);
    if entry.needs_api_key() {
        let key = secrets
            .get_api_key(provider_id)
            .map_err(|e| format!("could not read the keychain: {e}"))?
            .ok_or_else(|| {
                format!(
                    "{} has no API key yet. Add one in settings{}.",
                    entry.label,
                    entry
                        .api_key_url
                        .map(|u| format!(" — get one at {u}"))
                        .unwrap_or_default()
                )
            })?;
        config = config.with_api_key(providers::ApiKey::new(key));
    }

    OpenAiCompatible::new(config)
        .map(|p| Box::new(p) as Box<dyn Provider>)
        .map_err(|e| e.to_string())
}

/// Sends the cheapest possible real request so the user can confirm a key works
/// before relying on it mid-call. Required by the PRD's provider acceptance test.
#[tauri::command]
async fn test_provider(
    provider_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let provider = build_provider(&provider_id, &state.secrets)?;
    let request = ChatRequest {
        messages: vec![
            ChatMessage::system("Reply with the single word: ok"),
            ChatMessage::user("ping"),
        ],
        model: provider.model().to_string(),
        max_tokens: Some(16),
        temperature: Some(0.0),
    };
    let cancel = CancellationToken::new();
    let text = providers::collect_text(provider.stream_chat(request, cancel))
        .await
        .map_err(|e| e.to_string())?;
    if text.trim().is_empty() {
        return Err("the provider connected but returned no content".to_string());
    }
    Ok(text)
}

// ------------------------------------------------------------------- ask ------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskDelta {
    request_id: String,
    content: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskDone {
    request_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskError {
    request_id: String,
    message: String,
}

/// A passage that was actually put in front of the model, shown to the user so an
/// answer's grounding is inspectable rather than asserted.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskSource {
    path: String,
    section: Option<String>,
    excerpt: String,
    start_offset: usize,
    end_offset: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AskSources {
    request_id: String,
    /// False when the needs-retrieval gate decided the turn did not warrant a
    /// lookup, which is different from looking and finding nothing.
    searched: bool,
    sources: Vec<AskSource>,
}

/// Builds the grounded prompt for a question: retrieve, then render.
///
/// Returns the passages alongside the messages so the caller can show the user
/// exactly what grounded the answer.
fn build_messages(
    state: &AppState,
    prompt: &str,
    query_vector: Option<(&str, &[f32])>,
) -> Result<(Vec<ChatMessage>, bool, Vec<AskSource>), String> {
    // The gate keeps small talk from paying for a lookup.
    let searched = rag::needs_retrieval(prompt);
    let mut sources = Vec::new();
    let kb_context = if searched {
        let chunks =
            state.with_kb(|kb| kb.retrieve_hybrid(prompt, query_vector, RETRIEVAL_LIMIT))?;
        let mut buf = String::new();
        for c in &chunks {
            buf.push_str(&format!(
                "[{}{}]\n{}\n\n",
                c.path,
                c.section
                    .as_deref()
                    .map(|s| format!(" — {s}"))
                    .unwrap_or_default(),
                c.text
            ));
            sources.push(AskSource {
                path: c.path.clone(),
                section: c.section.clone(),
                excerpt: c.text.clone(),
                start_offset: c.start_offset,
                end_offset: c.end_offset,
            });
        }
        buf
    } else {
        String::new()
    };

    let bundle = match state.prompts.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    // Empty string, never None: "looked and found nothing" is a value the
    // shipped prompts know how to talk about, whereas None is a missing slot
    // and would fail to render.
    let vars = PromptVars {
        kb_context: Some(&kb_context),
        transcript: None,
        question: Some(prompt),
        profile: Profile::General,
    };
    let system = bundle
        .render(Mode::Ask, &vars, Tone::Neutral, Length::Normal)
        .map_err(|e| e.to_string())?;

    Ok((
        vec![ChatMessage::system(system), ChatMessage::user(prompt)],
        searched,
        sources,
    ))
}

#[tauri::command]
async fn ask_start(
    prompt: String,
    provider_id: String,
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    if prompt.trim().is_empty() {
        return Err("the question is empty".to_string());
    }

    let provider = build_provider(&provider_id, &state.secrets)?;

    // The query embedding for the vector arm — best-effort by design. A dead
    // embeddings endpoint must degrade the answer to keyword grounding, not
    // block it: the user asked a question, not for a healthy vector index.
    let query_embedding: Option<(String, Vec<f32>)> = match build_embeddings_client(&state) {
        Ok(Some(client)) if rag::needs_retrieval(&prompt) => {
            match client.embed(std::slice::from_ref(&prompt)).await {
                Ok(mut vectors) if !vectors.is_empty() => {
                    Some((client.model().to_string(), vectors.remove(0)))
                }
                Ok(_) => None,
                Err(error) => {
                    eprintln!("skia: query embedding failed, retrieval is keyword-only: {error}");
                    None
                }
            }
        }
        Ok(_) => None,
        Err(error) => {
            eprintln!("skia: embeddings misconfigured, retrieval is keyword-only: {error}");
            None
        }
    };

    let (messages, searched, sources) = build_messages(
        &state,
        &prompt,
        query_embedding
            .as_ref()
            .map(|(model, vector)| (model.as_str(), vector.as_slice())),
    )?;

    let request_id = format!("ask-{}", state.next_request.fetch_add(1, Ordering::Relaxed));
    let cancel = CancellationToken::new();
    match state.inflight.lock() {
        Ok(mut g) => {
            g.insert(request_id.clone(), cancel.clone());
        }
        Err(poisoned) => {
            poisoned
                .into_inner()
                .insert(request_id.clone(), cancel.clone());
        }
    }

    // Record the question now so history is accurate even if the answer fails.
    let session_id = state.with_store(|s| s.create_session("ask", Some(prompt.as_str())))?;
    state.with_store(|s| s.append_message(session_id, "user", &prompt))?;

    // Announce the grounding before any token arrives, so the user can see what
    // the answer is based on while it is still being written — and can see that
    // it is based on nothing, when that is the case.
    if let Err(e) = window.emit(
        "ask:sources",
        AskSources {
            request_id: request_id.clone(),
            searched,
            sources,
        },
    ) {
        eprintln!("skia: could not emit ask:sources: {e}");
    }

    let request = ChatRequest {
        messages,
        model: provider.model().to_string(),
        max_tokens: None,
        temperature: None,
    };

    let app = window.clone();
    let id = request_id.clone();
    let handle = window.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut stream = provider.stream_chat(request, cancel);
        let mut answer = String::new();
        let mut failure: Option<String> = None;

        while let Some(next) = stream.next().await {
            match next {
                Ok(delta) => {
                    answer.push_str(&delta.content);
                    if let Err(e) = app.emit(
                        "ask:delta",
                        AskDelta {
                            request_id: id.clone(),
                            content: delta.content,
                        },
                    ) {
                        // The window is gone; stop rather than spin.
                        eprintln!("skia: could not emit ask:delta: {e}");
                        break;
                    }
                }
                Err(e) => {
                    failure = Some(e.to_string());
                    break;
                }
            }
        }

        // Persist whatever arrived, then report. Partial answers are still worth
        // keeping; losing them silently would be worse.
        if let Some(state) = handle.try_state::<AppState>() {
            if !answer.is_empty() {
                if let Err(e) =
                    state.with_store(|s| s.append_message(session_id, "assistant", &answer))
                {
                    eprintln!("skia: could not save the answer: {e}");
                }
            }
            if let Err(e) = state.with_store(|s| s.end_session(session_id)) {
                eprintln!("skia: could not close the session: {e}");
            }
            match state.inflight.lock() {
                Ok(mut g) => {
                    g.remove(&id);
                }
                Err(poisoned) => {
                    poisoned.into_inner().remove(&id);
                }
            }
        }

        let emitted = match failure {
            Some(message) => app.emit(
                "ask:error",
                AskError {
                    request_id: id.clone(),
                    message,
                },
            ),
            None => app.emit(
                "ask:done",
                AskDone {
                    request_id: id.clone(),
                },
            ),
        };
        if let Err(e) = emitted {
            eprintln!("skia: could not emit the terminal ask event: {e}");
        }
    });

    Ok(request_id)
}

#[tauri::command]
fn ask_cancel(request_id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = match state.inflight.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    match guard.remove(&request_id) {
        Some(token) => {
            token.cancel();
            Ok(())
        }
        // Reported rather than silently accepted: the caller believes something
        // is running, and it is not.
        None => Err(format!("no such active request: {request_id}")),
    }
}

// -------------------------------------------------------------- windows ------

/// Opens the dashboard, creating nothing: it is declared hidden in the config so
/// it is warm by the time the user asks for it.
#[tauri::command]
fn open_dashboard(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(DASHBOARD_LABEL)
        .ok_or("no dashboard window — check tauri.conf.json")?;
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_dashboard(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window(DASHBOARD_LABEL) {
        Some(window) => window.hide().map_err(|e| e.to_string()),
        None => Err("no dashboard window".to_string()),
    }
}

#[tauri::command]
fn hide_overlay(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window(OVERLAY_LABEL) {
        Some(window) => window.hide().map_err(|e| e.to_string()),
        None => Err("no overlay window".to_string()),
    }
}

/// Resizes the overlay so it can grow to fit an answer and shrink back to a bar.
/// The frontend measures its own content; only it knows the right height.
#[tauri::command]
fn resize_overlay(height: f64, app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(OVERLAY_LABEL)
        .ok_or("no overlay window")?;
    let current = window.outer_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    // Clamped so a frontend bug cannot produce a window taller than the screen
    // or one too short to contain the input row.
    let clamped = height.clamp(64.0, 900.0);
    window
        .set_size(tauri::Size::Physical(tauri::PhysicalSize {
            width: current.width,
            height: (clamped * scale).round() as u32,
        }))
        .map_err(|e| e.to_string())
}

// -------------------------------------------------------------- prompts ------

#[tauri::command]
fn prompts_get(state: tauri::State<'_, AppState>) -> Result<PromptBundle, String> {
    let bundle = match state.prompts.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    Ok(bundle.clone())
}

#[tauri::command]
fn prompts_template(
    mode: Mode,
    profile: Profile,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let bundle = match state.prompts.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    Ok(bundle.template(mode, profile).to_string())
}

/// Saves an edited prompt. Rejects a template referring to variables Skia cannot
/// fill, so a broken prompt fails here rather than silently mid-call.
#[tauri::command]
fn prompts_set_override(
    mode: Mode,
    profile: Profile,
    template: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut bundle = match state.prompts.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    bundle
        .set_override(mode, profile, template)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn prompts_reset(
    mode: Mode,
    profile: Profile,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut bundle = match state.prompts.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    bundle.reset(mode, profile);
    Ok(())
}

// --------------------------------------------------------------- history -----

#[tauri::command]
fn history_sessions(limit: u32, state: tauri::State<'_, AppState>) -> Result<Vec<Session>, String> {
    state.with_store(|s| s.list_sessions(limit))
}

#[tauri::command]
fn history_messages(
    session_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Message>, String> {
    state.with_store(|s| s.messages_for_session(session_id))
}

#[tauri::command]
fn history_search(
    query: String,
    limit: u32,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Message>, String> {
    state.with_store(|s| s.search_messages(&query, limit))
}

// -------------------------------------------------------- knowledge base -----

#[tauri::command]
fn kb_ingest_file(path: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let outcome = state.with_kb(|kb| kb.ingest_file(std::path::Path::new(&path)))?;
    serde_json::to_string(&outcome).map_err(|e| e.to_string())
}

#[tauri::command]
fn kb_documents(state: tauri::State<'_, AppState>) -> Result<Vec<rag::Document>, String> {
    state.with_kb(|kb| kb.list_documents())
}

#[tauri::command]
fn kb_remove_document(path: String, state: tauri::State<'_, AppState>) -> Result<bool, String> {
    state.with_kb(|kb| kb.remove_document(&path))
}

// ---------------------------------------------------------------- meetings ----

/// A started meeting, with what Skia already knew walking in.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeetingStarted {
    meeting_id: i64,
    brief: storage::MeetingBrief,
}

/// One meeting with everything the UI shows about it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeetingDetail {
    meeting: storage::Meeting,
    attendees: Vec<storage::Person>,
    action_items: Vec<storage::ActionItem>,
}

/// Start a meeting and return the pre-meeting brief in the same call: the
/// brief's whole value is being on screen *before* the conversation starts.
#[tauri::command]
fn meeting_start(
    title: Option<String>,
    profile: String,
    attendees: Vec<storage::AttendeeSpec>,
    state: tauri::State<'_, AppState>,
) -> Result<MeetingStarted, String> {
    let meeting_id =
        state.with_store(|s| s.start_meeting(title.as_deref(), &profile, &attendees))?;
    let people = state.with_store(|s| s.meeting_attendees(meeting_id))?;
    let ids: Vec<i64> = people.iter().map(|p| p.id).collect();
    let brief = state.with_store(|s| s.brief_for_people(&ids, Some(meeting_id)))?;
    Ok(MeetingStarted { meeting_id, brief })
}

#[tauri::command]
fn meeting_end(meeting_id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.with_store(|s| s.end_meeting(meeting_id))
}

#[tauri::command]
fn meetings_list(state: tauri::State<'_, AppState>) -> Result<Vec<storage::Meeting>, String> {
    state.with_store(|s| s.list_meetings())
}

#[tauri::command]
fn meeting_detail(
    meeting_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<MeetingDetail, String> {
    let meeting = state
        .with_store(|s| s.list_meetings())?
        .into_iter()
        .find(|m| m.id == meeting_id)
        .ok_or_else(|| format!("there is no meeting with id {meeting_id}"))?;
    Ok(MeetingDetail {
        attendees: state.with_store(|s| s.meeting_attendees(meeting_id))?,
        action_items: state.with_store(|s| s.meeting_action_items(meeting_id))?,
        meeting,
    })
}

#[tauri::command]
fn meeting_add_action(
    meeting_id: i64,
    person_id: Option<i64>,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<i64, String> {
    state.with_store(|s| s.add_action_item(meeting_id, person_id, &text))
}

#[tauri::command]
fn meeting_set_action_done(
    item_id: i64,
    done: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.with_store(|s| s.set_action_done(item_id, done))
}

/// Append a line to the meeting's transcript.
///
/// Today this is typed notes; when live transcription lands it is finalized
/// utterances through the identical path — which is the point: the notes
/// feature is the transcript pipeline, exercised end to end before any STT
/// exists to feed it.
#[tauri::command]
fn meeting_append_note(
    meeting_id: i64,
    speaker: Option<String>,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state.with_kb(|kb| {
        kb.append_transcript_window(meeting_id, speaker.as_deref(), &text)
            .map(|_| ())
    })
}

/// Search one meeting's transcript — the meeting-scoped view that generic Ask
/// deliberately does not have.
#[tauri::command]
fn meeting_search(
    meeting_id: i64,
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<rag::RetrievedChunk>, String> {
    state.with_kb(|kb| kb.retrieve_meeting(meeting_id, &query, RETRIEVAL_LIMIT))
}

// ------------------------------------------------------- semantic index ------

/// The embeddings configuration as the UI shows it, coverage included.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SemanticStatus {
    /// `None` when semantic search is off and retrieval is keyword-only.
    provider_id: Option<String>,
    model: Option<String>,
    embedded: u64,
    total: u64,
}

/// Read the embeddings settings, if any are set.
fn embeddings_settings(state: &AppState) -> Result<Option<(String, String)>, String> {
    let provider = state.with_store(|s| s.get_setting(KEY_EMBEDDINGS_PROVIDER))?;
    let model = state.with_store(|s| s.get_setting(KEY_EMBEDDINGS_MODEL))?;
    Ok(match (provider, model) {
        (Some(provider), Some(model)) if !provider.is_empty() && !model.is_empty() => {
            Some((provider, model))
        }
        _ => None,
    })
}

/// Build the embeddings client for the configured provider, or `None` when
/// semantic search is off. Errors are configuration problems worth showing —
/// a missing key, an unknown provider — not the absence of configuration.
fn build_embeddings_client(state: &AppState) -> Result<Option<EmbeddingsClient>, String> {
    let Some((provider_id, model)) = embeddings_settings(state)? else {
        return Ok(None);
    };
    let entry = catalog::entry(&provider_id)
        .ok_or_else(|| format!("unknown embeddings provider: {provider_id}"))?;

    let mut config = EmbeddingsConfig::new(entry.id, entry.base_url, model);
    if entry.needs_api_key() {
        let key = state
            .secrets
            .get_api_key(entry.id)
            .map_err(|e| format!("could not read the keychain: {e}"))?
            .ok_or_else(|| {
                format!(
                    "{} has no API key yet — add one in Providers before using it for embeddings",
                    entry.label
                )
            })?;
        config = config.with_api_key(providers::ApiKey::new(key));
    }
    EmbeddingsClient::new(config)
        .map(Some)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn kb_semantic_status(state: tauri::State<'_, AppState>) -> Result<SemanticStatus, String> {
    match embeddings_settings(&state)? {
        Some((provider_id, model)) => {
            let coverage = state.with_kb(|kb| kb.embedding_coverage(&model))?;
            Ok(SemanticStatus {
                provider_id: Some(provider_id),
                model: Some(model),
                embedded: coverage.embedded,
                total: coverage.total,
            })
        }
        None => {
            let coverage = state.with_kb(|kb| kb.embedding_coverage(""))?;
            Ok(SemanticStatus {
                provider_id: None,
                model: None,
                embedded: 0,
                total: coverage.total,
            })
        }
    }
}

/// Turn semantic search on (a provider id) or off (`None`).
///
/// Validation happens now, not at first use: the client is built — which
/// checks the base URL, the model, and that the key exists — so a missing key
/// is an error at the moment of choice rather than a silent keyword-only
/// downgrade discovered days later.
#[tauri::command]
fn kb_set_embeddings_provider(
    provider_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<SemanticStatus, String> {
    match provider_id {
        None => {
            state.with_store(|s| s.set_setting(KEY_EMBEDDINGS_PROVIDER, ""))?;
            state.with_store(|s| s.set_setting(KEY_EMBEDDINGS_MODEL, ""))?;
        }
        Some(provider_id) => {
            let entry = catalog::entry(&provider_id)
                .ok_or_else(|| format!("unknown provider: {provider_id}"))?;
            let model = entry.embedding_model.ok_or_else(|| {
                format!("{} offers no embedding model to default to", entry.label)
            })?;
            state.with_store(|s| s.set_setting(KEY_EMBEDDINGS_PROVIDER, entry.id))?;
            state.with_store(|s| s.set_setting(KEY_EMBEDDINGS_MODEL, model))?;
            // Validate the whole path now; roll back rather than store a
            // configuration that can only fail later.
            if let Err(error) = build_embeddings_client(&state) {
                state.with_store(|s| s.set_setting(KEY_EMBEDDINGS_PROVIDER, ""))?;
                state.with_store(|s| s.set_setting(KEY_EMBEDDINGS_MODEL, ""))?;
                return Err(error);
            }
        }
    }
    kb_semantic_status(state)
}

/// Progress of one embedding pass.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbedProgress {
    embedded_now: usize,
    remaining: u64,
}

/// Embed one batch of pending chunks. The UI loops this until `remaining`
/// reaches zero, so one stuck call never holds a giant ingest hostage.
#[tauri::command]
async fn kb_embed_pending(state: tauri::State<'_, AppState>) -> Result<EmbedProgress, String> {
    let Some(client) = build_embeddings_client(&state)? else {
        return Err("semantic search is not configured".to_string());
    };
    let model = client.model().to_string();

    let pending = state.with_kb(|kb| kb.unembedded_chunks(&model, EMBED_BATCH))?;
    if pending.is_empty() {
        let coverage = state.with_kb(|kb| kb.embedding_coverage(&model))?;
        return Ok(EmbedProgress {
            embedded_now: 0,
            remaining: coverage.total - coverage.embedded,
        });
    }

    let texts: Vec<String> = pending.iter().map(|(_, text)| text.clone()).collect();
    let vectors = client.embed(&texts).await.map_err(|e| e.to_string())?;

    // The client guarantees order; zip is therefore correct by contract.
    let embedded_now = vectors.len();
    for ((chunk_id, _), vector) in pending.iter().zip(vectors) {
        state.with_kb(|kb| kb.store_embedding(*chunk_id, &model, &vector))?;
    }

    let coverage = state.with_kb(|kb| kb.embedding_coverage(&model))?;
    Ok(EmbedProgress {
        embedded_now,
        remaining: coverage.total - coverage.embedded,
    })
}

// ------------------------------------------------------- backup / restore ----

/// Random per install, so two backups can be told apart without identifying
/// the machine — see the manifest's own note on this.
const KEY_DEVICE_ID: &str = "sync.device_id";
/// The last generation this device wrote, so a new backup supersedes it.
const KEY_BACKUP_GENERATION: &str = "sync.backup_generation";

/// Read (creating on first use) this install's device id.
///
/// Derived from a timestamp and the process id rather than a hardware
/// identifier: it only has to be unlikely to collide with another install's,
/// and anything stronger would be collecting more than the job needs.
fn device_id(state: &AppState) -> Result<String, String> {
    if let Some(existing) = state.with_store(|s| s.get_setting(KEY_DEVICE_ID))? {
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let fresh = format!("{nanos:x}-{:x}", std::process::id());
    state.with_store(|s| s.set_setting(KEY_DEVICE_ID, &fresh))?;
    Ok(fresh)
}

/// Write a snapshot and manifest into `directory`.
///
/// Blocking task: `VACUUM INTO` on a large database takes real time, and
/// holding a command thread through it would stall every other IPC call.
#[tauri::command]
async fn backup_now(
    directory: String,
    state: tauri::State<'_, AppState>,
) -> Result<sync::BackupOutcome, String> {
    let device = device_id(&state)?;
    let generation = state
        .with_store(|s| s.get_setting(KEY_BACKUP_GENERATION))?
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(0);

    let outcome = state.with_store(|store| {
        // `SyncError` is not a `StoreError`, so the mapping happens here rather
        // than being smuggled through `with_store`'s signature.
        Ok(sync::back_up(
            store,
            std::path::Path::new(&directory),
            &device,
            generation,
        ))
    })?;
    let outcome = outcome.map_err(|e| e.to_string())?;

    state.with_store(|s| {
        s.set_setting(
            KEY_BACKUP_GENERATION,
            &outcome.manifest.generation.to_string(),
        )
    })?;
    Ok(outcome)
}

/// Validate a backup and queue it for the next launch.
///
/// Two steps on purpose. Validating now means a wrong folder or a damaged
/// snapshot is refused while the user is still looking at the dialog; applying
/// at startup means the swap happens when nothing holds the database open.
#[tauri::command]
fn restore_request(
    directory: String,
    app: tauri::AppHandle,
    _state: tauri::State<'_, AppState>,
) -> Result<sync::Manifest, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    sync::request_restore(std::path::Path::new(&directory), &data_dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn restore_cancel(app: tauri::AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    sync::cancel_restore(&data_dir).map_err(|e| e.to_string())
}

/// The backup queued for the next launch, if any.
#[tauri::command]
fn restore_pending(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(sync::pending_restore(&data_dir).map(|p| p.display().to_string()))
}

/// What the last startup's restore did, if one was applied. Read once by the
/// UI so a completed restore is confirmed rather than silently assumed.
#[tauri::command]
fn restore_report(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    state.with_store(|s| s.get_setting(KEY_RESTORE_REPORT))
}

/// Set at startup by the restore that ran, cleared once the UI has shown it.
const KEY_RESTORE_REPORT: &str = "sync.last_restore";

#[tauri::command]
fn restore_report_clear(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.with_store(|s| s.set_setting(KEY_RESTORE_REPORT, ""))
}

// --------------------------------------------------------------- privacy -----

/// Exports everything on device. Covers both schemas in the database: leaving
/// the knowledge base out of an "export everything" would be a quiet lie.
#[tauri::command]
fn export_data(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let history: serde_json::Value =
        serde_json::from_str(&state.with_store(|s| s.export_json())?).map_err(|e| e.to_string())?;
    let documents = state.with_kb(|kb| kb.list_documents())?;
    serde_json::to_string_pretty(&serde_json::json!({
        "history": history,
        "knowledgeBase": documents,
    }))
    .map_err(|e| e.to_string())
}

/// Deletes everything on device, both schemas in the database.
#[tauri::command]
fn purge_data(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.with_store(|s| s.purge_all())?;
    state.with_kb(|kb| kb.purge_all())?;
    Ok(())
}

// ------------------------------------------------------------------ audio ----

#[tauri::command]
fn audio_devices() -> Result<Vec<audio::DeviceInfo>, String> {
    audio::list_devices().map_err(|e| e.to_string())
}

#[tauri::command]
fn audio_status(state: tauri::State<'_, AppState>) -> Result<audio::AudioStatus, String> {
    state.audio.status().map_err(|e| e.to_string())
}

/// Async over a blocking task because the first call ever may put the OS
/// microphone-consent dialog on screen and wait minutes for a human. The
/// consent step is load-bearing, not politeness: cpal reaches the microphone
/// through the CoreAudio HAL, which never triggers the dialog on its own —
/// macOS just delivers silence until someone asks. See `audio::consent`.
#[tauri::command]
async fn audio_meter_start(
    state: tauri::State<'_, AppState>,
) -> Result<audio::AudioStatus, String> {
    let handle = state.audio.clone();
    tauri::async_runtime::spawn_blocking(move || {
        audio::ensure_microphone().map_err(|e| e.to_string())?;
        handle.meter_start().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn audio_meter_stop(state: tauri::State<'_, AppState>) -> Result<audio::AudioStatus, String> {
    state.audio.meter_stop().map_err(|e| e.to_string())
}

/// Record a short 16 kHz mono WAV and return where it landed.
///
/// Async over a blocking task because the recording takes as long as it takes:
/// blocking a command thread for five seconds would freeze every other IPC
/// call, including the level meter events the user watches while recording.
#[tauri::command]
async fn audio_probe(
    seconds: Option<f32>,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<audio::ProbeOutcome, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("probes");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("mic-probe-{stamp}.wav"));

    let handle = state.audio.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Same consent gate as the meter: without it this records silence.
        audio::ensure_microphone().map_err(|e| e.to_string())?;
        handle
            .probe(seconds.unwrap_or(5.0), path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ------------------------------------------------------------------ setup ----

/// The file builds up to 0.1.0 wrote the knowledge base to, before both schemas
/// were put in `skia.db` where their design always assumed they were.
const LEGACY_KB_FILE: &str = "skia-kb.db";

/// Carry a pre-0.1.0 knowledge base into the shared database, then move the old
/// file aside.
///
/// Renamed rather than deleted. The rename is what stops the adoption being
/// attempted on every subsequent launch, and keeping the bytes costs a few
/// megabytes of the user's own documents that Skia has no business destroying
/// on their behalf — they can remove it whenever they like.
///
/// Nothing here is fatal. A knowledge base that could not be carried across is
/// an empty knowledge base plus a file still on disk, which is recoverable; a
/// Skia that refuses to launch over it is not. So every branch reports and
/// returns, in the same fail-closed spirit as the capture-exclusion status:
/// state what actually happened, never what was intended.
fn adopt_legacy_knowledge_base(kb: &KnowledgeBase, dir: &std::path::Path) {
    let legacy = dir.join(LEGACY_KB_FILE);

    match kb.adopt_legacy(&legacy) {
        Ok(rag::Adoption::NothingToAdopt) => {}
        Ok(rag::Adoption::Adopted { documents, chunks }) => {
            eprintln!(
                "skia: moved {documents} document(s) and {chunks} chunk(s) from \
                 {LEGACY_KB_FILE} into skia.db"
            );
            // Also move the WAL and shared-memory sidecars, or SQLite would
            // find a journal for a database that is no longer beside it.
            for suffix in ["", "-wal", "-shm"] {
                let from = dir.join(format!("{LEGACY_KB_FILE}{suffix}"));
                if !from.exists() {
                    continue;
                }
                let to = dir.join(format!("{LEGACY_KB_FILE}{suffix}.migrated"));
                if let Err(e) = std::fs::rename(&from, &to) {
                    eprintln!(
                        "skia: {} was adopted but could not be renamed, so it will \
                         be skipped rather than re-read next launch: {e}",
                        from.display()
                    );
                }
            }
        }
        Ok(rag::Adoption::AlreadyPopulated { documents }) => {
            eprintln!(
                "skia: {LEGACY_KB_FILE} still holds a knowledge base, but skia.db \
                 already has {documents} document(s), so it was left untouched \
                 rather than merged — re-add anything missing from the dashboard"
            );
        }
        Err(e) => eprintln!("skia: {LEGACY_KB_FILE} could not be adopted: {e}"),
    }
}

fn toggle_overlay(window: &WebviewWindow) -> tauri::Result<()> {
    if window.is_visible()? {
        window.hide()
    } else {
        window.show()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    // The panel plugin owns the registry `to_panel` inserts into, so it has to
    // be present before setup runs. macOS-only, because NSPanel is.
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .setup(|app| {
            // The macOS activation policy is deliberately NOT set here. See the
            // RunEvent::Ready handler in `run()`: the overlay has to exist as a
            // non-activating panel and be ordered front before the app may be
            // demoted, and doing it this early was measured at 0/5 on screen.
            let dir = app.path().app_data_dir()?;

            // One file, both schemas. `storage` owns `PRAGMA user_version` and
            // `rag` carries its own version in `kb_meta` with every table
            // prefixed `kb_`, precisely so they can share it — see the module
            // documentation on both. Two connections rather than one because a
            // `rusqlite::Connection` is not `Sync`; WAL and `busy_timeout = 5000`
            // are what make that safe.
            //
            // It matters beyond tidiness: everything the user would want backed
            // up or restored is then a single snapshot, taken with `VACUUM INTO`
            // while the app keeps running. Two files would need a generation
            // counter to stay consistent with each other.
            let database = dir.join("skia.db");

            // A requested restore is applied here, before anything opens the
            // database — the only moment nothing holds a handle to it. See
            // `sync::request_restore` for why it cannot happen mid-session.
            let restore_report = match sync::apply_pending(&dir, &database) {
                Some(Ok(outcome)) => {
                    eprintln!(
                        "skia: restored a backup from {} (generation {}); the previous \
                         database was kept at {}",
                        outcome.manifest.device_id,
                        outcome.manifest.generation,
                        outcome.displaced_to
                    );
                    Some(format!(
                        "Restored the backup written {} by device {} (generation {}). Your \
                         previous data was kept at {}. API keys are not part of a backup, so \
                         re-enter them in Providers if this is a new machine.",
                        outcome.manifest.created_at,
                        outcome.manifest.device_id,
                        outcome.manifest.generation,
                        outcome.displaced_to
                    ))
                }
                Some(Err(error)) => {
                    // Reported, never fatal: a failed restore must leave a
                    // usable app, and the marker has already been cleared so
                    // it cannot fail on every launch.
                    eprintln!("skia: the requested restore failed: {error}");
                    Some(format!("The requested restore failed: {error}"))
                }
                None => None,
            };

            let store = Store::open(&database)?;
            if let Some(report) = &restore_report {
                // Written into the freshly restored database on purpose: the
                // note belongs to the data the user is now looking at.
                if let Err(e) = store.set_setting(KEY_RESTORE_REPORT, report) {
                    eprintln!("skia: the restore outcome could not be recorded: {e}");
                }
            }
            let kb = KnowledgeBase::open(&database)?;
            adopt_legacy_knowledge_base(&kb, &dir);
            let requested = read_capture_preference(&store)?;
            let needs_setup = !read_onboarding_done(&store)?;

            // The audio engine forwards its events to every window: the
            // dashboard renders the meter today, and the overlay will want the
            // same signal when live mode lands.
            let audio = audio::Handle::spawn({
                let handle = app.handle().clone();
                move |event| {
                    let result = match event {
                        audio::EngineEvent::Level(level) => handle.emit("audio:level", *level),
                        audio::EngineEvent::Status(status) => {
                            handle.emit("audio:status", status.clone())
                        }
                    };
                    if let Err(e) = result {
                        eprintln!("skia: could not emit an audio event: {e}");
                    }
                }
            });

            app.manage(AppState {
                store: Mutex::new(store),
                kb: Mutex::new(kb),
                secrets: SecretStore::new(KEYCHAIN_SERVICE),
                prompts: Mutex::new(PromptBundle::shipped_defaults()),
                inflight: Mutex::new(HashMap::new()),
                next_request: AtomicU64::new(1),
                audio: std::sync::Arc::new(audio),
            });

            let window = app
                .get_webview_window(OVERLAY_LABEL)
                .ok_or("no window labelled overlay — check tauri.conf.json")?;

            let status = stealth::apply(&window, requested)?;
            if requested && !status.capture_exclusion.active {
                eprintln!(
                    "skia: capture exclusion is NOT active on this platform — {}",
                    status.capture_exclusion.guarantee
                );
            }

            register_toggle_shortcut(app.handle())?;

            // On a first run the overlay alone is a dead end: there is no
            // provider configured, so asking anything fails. Bring the dashboard
            // up so setup is the first thing seen. Stashed for RunEvent::Ready,
            // because showing a window this early does not stick.
            if needs_setup {
                app.manage(NeedsSetup);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            stealth_status,
            set_capture_exclusion,
            providers_list,
            role_defaults,
            set_api_key,
            delete_api_key,
            test_provider,
            ask_start,
            ask_cancel,
            history_sessions,
            history_messages,
            history_search,
            kb_ingest_file,
            kb_documents,
            kb_remove_document,
            kb_semantic_status,
            kb_set_embeddings_provider,
            kb_embed_pending,
            meeting_start,
            meeting_end,
            meetings_list,
            meeting_detail,
            meeting_add_action,
            meeting_set_action_done,
            meeting_append_note,
            meeting_search,
            backup_now,
            restore_request,
            restore_cancel,
            restore_pending,
            restore_report,
            restore_report_clear,
            open_dashboard,
            hide_dashboard,
            hide_overlay,
            resize_overlay,
            prompts_get,
            prompts_template,
            prompts_set_override,
            prompts_reset,
            onboarding_done,
            set_onboarding_done,
            export_data,
            purge_data,
            audio_devices,
            audio_status,
            audio_meter_start,
            audio_meter_stop,
            audio_probe
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|handle, event| {
            // Order the overlay onto the screen once the app is actually ready.
            //
            // This cannot go in `setup()`. An app with an accessory activation
            // policy is never activated by the system on launch, so nothing calls
            // `makeKeyAndOrderFront` and the window is created with correct
            // geometry but never appears. Doing it from `setup()` is too early:
            // macOS ignores the activation, and the result is intermittent — it
            // measured 2 of 3 cold launches via `SCShareableContent`, which is
            // worse than a clean failure. `RunEvent::Ready` fires after the app
            // has finished launching, where activation sticks.
            //
            // `show()` alone is not enough; the app itself has to be activated,
            // which is what `set_focus` does. That is exactly why `Presence`
            // reports `never_steals_focus: false`. An overlay nobody can see is
            // worse than one that activates once, and having both requires a
            // genuinely non-activating NSPanel, which is not built yet.
            if matches!(event, tauri::RunEvent::Ready) {
                match handle.get_webview_window(OVERLAY_LABEL) {
                    Some(window) => {
                        // Show first, convert second. The history here is
                        // measured and worth keeping, because it is what makes
                        // the ordering non-obvious:
                        //
                        //   Accessory in setup()             → 0/5 on screen.
                        //     tao applies the policy and then calls
                        //     `activateIgnoringOtherApps` during launch, so the
                        //     app has already opted out of activation by the
                        //     time that runs, and the window is never ordered
                        //     front.
                        //   Accessory after show+focus       → 0/5 on screen.
                        //     Demoting an app whose ordinary window is already
                        //     visible takes that window off the screen.
                        //   No accessory policy              → 5/5 on screen.
                        //
                        // In both failing cases `CGWindowListCopyWindowInfo`
                        // still listed the window with correct bounds while
                        // `SCShareableContent` reported `onScreen=false` — it
                        // existed and was invisible, the worst outcome for an
                        // overlay.
                        //
                        // The tension was never really "dock icon versus
                        // visibility"; it was that an *ordinary* window can only
                        // be ordered front by activating the app. A
                        // non-activating `NSPanel` can be ordered front
                        // regardless, so `panel::convert` orders it and only
                        // then demotes the app — and only if the panel reports
                        // itself on screen. See `panel.rs`.
                        if let Err(e) = window.show() {
                            eprintln!("skia: the overlay could not be brought on screen: {e}");
                        }
                        let outcome = panel::convert(&window);
                        if !outcome.fully_applied() {
                            // Not fatal: the app falls back to the previously
                            // shipped behaviour, and `stealth.rs` reports it.
                            // Focus is taken once here only in that fallback,
                            // because an invisible overlay is worse.
                            if let Err(e) = window.set_focus() {
                                eprintln!("skia: the overlay could not take focus: {e}");
                            }
                        }
                    }
                    None => eprintln!("skia: no overlay window to show"),
                }

                // First run: the overlay alone is a dead end with no provider
                // configured, so put setup in front of the user.
                if handle.try_state::<NeedsSetup>().is_some() {
                    match handle.get_webview_window(DASHBOARD_LABEL) {
                        Some(dashboard) => {
                            if let Err(e) = dashboard.show().and_then(|()| dashboard.set_focus()) {
                                eprintln!("skia: could not open first-run setup: {e}");
                            }
                        }
                        None => eprintln!("skia: no dashboard window for first-run setup"),
                    }
                }
            }
        });
}

fn register_toggle_shortcut(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, ShortcutState};

    let shortcut: tauri_plugin_global_shortcut::Shortcut = DEFAULT_TOGGLE_SHORTCUT.parse()?;

    app.plugin(
        ShortcutBuilder::new()
            .with_handler(move |app, _shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
                    if let Err(e) = toggle_overlay(&window) {
                        eprintln!("skia: failed to toggle overlay: {e}");
                    }
                }
            })
            .build(),
    )?;

    app.global_shortcut().register(shortcut)?;
    Ok(())
}
