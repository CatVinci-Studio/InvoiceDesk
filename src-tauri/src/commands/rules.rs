//! The classification rule set.

use crate::classify::{defaults, Rule};
use crate::commands::AppState;
use crate::db;
use tauri::State;

#[tauri::command]
pub fn list_rules(state: State<'_, AppState>) -> Result<Vec<Rule>, String> {
    db::load_rules(&state.lock())
}

#[tauri::command]
pub fn save_rule(state: State<'_, AppState>, rule: Rule) -> Result<i64, String> {
    db::save_rule(&state.lock(), &rule)
}

#[tauri::command]
pub fn delete_rule(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::delete_rule(&state.lock(), id)
}

/// Every category any rule produces, plus 未分类. This is the list the
/// category picker and the AI suggestion prompt both draw from, so a category
/// can never exist in one and not the other.
#[tauri::command]
pub fn list_categories(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let mut categories: Vec<String> = db::load_rules(&state.lock())?
        .into_iter()
        .map(|r| r.category)
        .collect();
    categories.sort();
    categories.dedup();
    if !categories
        .iter()
        .any(|c| c == crate::classify::UNCATEGORISED)
    {
        categories.push(crate::classify::UNCATEGORISED.to_string());
    }
    Ok(categories)
}

/// Puts back any built-in rule the user does not currently have.
///
/// Explicitly invoked from the rule editor, never on startup: an app that
/// resurrected deleted rules on its own would keep re-breaking whatever the
/// user fixed by deleting them.
#[tauri::command]
pub fn restore_default_rules(state: State<'_, AppState>) -> Result<usize, String> {
    let connection = state.lock();
    let existing = db::load_rules(&connection)?;
    let mut added = 0;
    for rule in defaults::rules() {
        let already = existing.iter().any(|e| {
            e.category == rule.category
                && e.conditions.len() == rule.conditions.len()
                && e.conditions
                    .iter()
                    .zip(&rule.conditions)
                    .all(|(a, b)| a.field == b.field && a.keywords == b.keywords)
        });
        if !already {
            db::save_rule(&connection, &rule)?;
            added += 1;
        }
    }
    Ok(added)
}

/// The rule set as JSON, for backing up or sharing with a colleague. Rule
/// sets are the asset a user builds up over months, so they have to be
/// portable.
#[tauri::command]
pub fn export_rules(state: State<'_, AppState>) -> Result<String, String> {
    let rules = db::load_rules(&state.lock())?;
    serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())
}

/// Adds rules from an export. Imported rules get new ids rather than
/// overwriting whatever happens to occupy theirs.
#[tauri::command]
pub fn import_rules(state: State<'_, AppState>, json: String) -> Result<usize, String> {
    let rules: Vec<Rule> =
        serde_json::from_str(&json).map_err(|e| format!("规则文件无法解析：{e}"))?;
    let connection = state.lock();
    for rule in &rules {
        db::save_rule(
            &connection,
            &Rule {
                id: None,
                ..rule.clone()
            },
        )?;
    }
    Ok(rules.len())
}

/// Writes the rule set to a file the user picked.
///
/// The path comes from the frontend's native save dialog, but the WRITING
/// happens here - the same split the report export already uses. The
/// alternative would be adding `@tauri-apps/plugin-fs` just so the frontend
/// could write one text file, which means a new dependency and a new
/// filesystem permission in the capability manifest, for a job the backend
/// can already do.
#[tauri::command]
pub fn export_rules_to_file(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    let rules = db::load_rules(&state.lock())?;
    let json = serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("无法写入文件：{e}"))?;
    Ok(rules.len())
}

/// Reads a rule set the user picked and adds it. Same import semantics as
/// [`import_rules`]: new ids, nothing overwritten.
#[tauri::command]
pub fn import_rules_from_file(state: State<'_, AppState>, path: String) -> Result<usize, String> {
    let json = std::fs::read_to_string(&path).map_err(|e| format!("无法读取文件：{e}"))?;
    import_rules(state, json)
}
