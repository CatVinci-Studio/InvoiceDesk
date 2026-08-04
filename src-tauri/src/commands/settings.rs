//! Settings, and the AI credentials that deliberately do not live with them.
//!
//! See [`crate::ai::route::AiSettings`] for why keys are stored apart from
//! the ledger.

use crate::ai::route::AiSettings;
use crate::commands::AppState;
use crate::db;
use tauri::{AppHandle, State};

const AI_KEY: &str = "ai";

/// Reads the AI settings, falling back to the (network-off) defaults.
///
/// Never fails: a corrupt or absent settings row means the safe default, not
/// a broken import. The one thing it must never do is fail *open* - a parse
/// error that silently enabled uploads would be a privacy bug.
pub fn load_ai_settings(state: &AppState) -> AiSettings {
    db::get_setting(&state.lock(), AI_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_ai_settings(state: State<'_, AppState>) -> AiSettings {
    load_ai_settings(&state)
}

#[tauri::command]
pub fn set_ai_settings(state: State<'_, AppState>, settings: AiSettings) -> Result<(), String> {
    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    db::set_setting(&state.lock(), AI_KEY, &json)
}

/// A free-form key/value pair for UI preferences (last used report, column
/// widths). Kept out of [`AiSettings`] so the typed settings stay a
/// deliberate, reviewed surface rather than a bag.
#[tauri::command]
pub fn get_preference(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    db::get_setting(&state.lock(), &format!("pref.{key}"))
}

#[tauri::command]
pub fn set_preference(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    db::set_setting(&state.lock(), &format!("pref.{key}"), &value)
}

/// Checks that the configured provider actually answers, and reports what is
/// wrong in words the user can act on.
#[tauri::command]
pub async fn test_ai_connection(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let settings = load_ai_settings(&state);
    let endpoint = crate::ai::route::resolve(&app, &settings, crate::ai::route::Purpose::Text)?;
    aicompat::providers::custom::list_models(&endpoint.base_url, endpoint.api_key.as_deref())
        .await
        .map(|models| format!("连接成功，可用模型 {} 个", models.len()))
        .map_err(|e| format!("连接失败：{e}"))
}

/// The models a provider reports, for the picker.
#[tauri::command]
pub async fn list_provider_models(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let settings = load_ai_settings(&state);
    let endpoint = crate::ai::route::resolve(&app, &settings, crate::ai::route::Purpose::Text)?;
    aicompat::providers::custom::list_models(&endpoint.base_url, endpoint.api_key.as_deref()).await
}
