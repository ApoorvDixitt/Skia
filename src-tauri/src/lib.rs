// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

mod stealth;
pub mod storage;

use std::sync::Mutex;

use tauri::{Manager, WebviewWindow};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use stealth::StealthStatus;
use storage::Store;

/// The default overlay hotkey. Silent by design: registering a global shortcut
/// produces no sound, banner, or notification on either platform.
const DEFAULT_TOGGLE_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "cmd+shift+space"
} else {
    "ctrl+shift+space"
};

/// Settings key for the user's capture-exclusion preference.
const KEY_CAPTURE_EXCLUSION: &str = "stealth.capture_exclusion_requested";

/// `rusqlite::Connection` is not `Sync`, so the store is behind a mutex.
struct AppState {
    store: Mutex<Store>,
}

impl AppState {
    /// Runs `f` against the store. A poisoned lock means another thread panicked
    /// mid-operation; the connection itself is still usable, and refusing to
    /// read the user's own data would be worse than continuing.
    fn with_store<T>(
        &self,
        f: impl FnOnce(&Store) -> Result<T, storage::StoreError>,
    ) -> Result<T, String> {
        let guard = match self.store.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&guard).map_err(|e| e.to_string())
    }
}

/// Reads the persisted preference. Defaults to enabled, since an invisible
/// overlay is the point — but the UI is responsible for stating how far that
/// can actually be trusted on the current platform.
fn read_capture_preference(store: &Store) -> Result<bool, storage::StoreError> {
    Ok(store
        .get_setting(KEY_CAPTURE_EXCLUSION)?
        .map(|v| v == "true")
        .unwrap_or(true))
}

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

/// Exports everything on device as JSON. Required by the privacy commitment in
/// the PRD: the user can always take their data out.
#[tauri::command]
fn export_data(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state.with_store(|store| store.export_json())
}

/// Deletes everything on device. The other half of the privacy commitment.
#[tauri::command]
fn purge_data(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.with_store(|store| store.purge_all())
}

/// Toggles overlay visibility. With no dock icon and no taskbar entry, the
/// hotkey is the primary way to summon and dismiss the window.
fn toggle_overlay(window: &WebviewWindow) -> tauri::Result<()> {
    if window.is_visible()? {
        window.hide()
    } else {
        window.show()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // No dock icon on macOS. Presence invisibility is supported on every
            // platform we target, unlike capture exclusion.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Everything is stored on device, under the OS app-data directory.
            let db_path = app.path().app_data_dir()?.join("skia.db");
            let store = Store::open(&db_path)?;
            let requested = read_capture_preference(&store)?;
            app.manage(AppState {
                store: Mutex::new(store),
            });

            let window = app
                .get_webview_window("main")
                .ok_or("no window labelled 'main' — check tauri.conf.json")?;

            // Apply stealth up front so the overlay is never briefly capturable
            // between launch and the frontend asking for status.
            let status = stealth::apply(&window, requested)?;
            if requested && !status.capture_exclusion.active {
                // Surfaced, not swallowed: the user needs to know the overlay is
                // visible in screen shares on this system.
                eprintln!(
                    "skia: capture exclusion is NOT active on this platform — {}",
                    status.capture_exclusion.guarantee
                );
            }

            register_toggle_shortcut(app.handle())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            stealth_status,
            set_capture_exclusion,
            export_data,
            purge_data
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn register_toggle_shortcut(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, ShortcutState};

    let shortcut: tauri_plugin_global_shortcut::Shortcut = DEFAULT_TOGGLE_SHORTCUT.parse()?;

    app.plugin(
        ShortcutBuilder::new()
            .with_handler(move |app, _shortcut, event| {
                // Fire on press only, otherwise the overlay toggles twice.
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                if let Some(window) = app.get_webview_window("main") {
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
