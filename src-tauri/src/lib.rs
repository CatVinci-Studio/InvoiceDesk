//! InvoiceDesk - 发票扫描、分类统计与报销表生成。
//!
//! The crate is split so the parts that handle money are pure functions over
//! bytes, and everything touching the network, the disk or the window lives
//! at the edges:
//!
//! - [`model`]     the invoice, and the rule that money is integer 分
//! - [`extract`]   bytes → fields, layered by how far each source can be trusted
//! - [`parse`]     text → fields, and the cross-field checks
//! - [`classify`]  fields → 报销类别, by rule first and by model only as a fallback
//! - [`db`]        the local ledger, and duplicate-reimbursement detection
//! - [`report`]    invoices → an .xlsx reimbursement sheet
//! - [`ai`]        provider catalog, credential routing, vision recognition
//! - [`commands`]  the Tauri surface the frontend calls

pub mod ai;
pub mod auth;
pub mod classify;
pub mod commands;
pub mod db;
pub mod extract;
pub mod model;
pub mod parse;
pub mod report;

use commands::AppState;
use tauri::Manager;

/// Where the ledger lives: the platform's app-data directory, so it survives
/// upgrades and is included in whatever the user already backs up.
fn database_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("找不到数据目录：{e}"))?;
    Ok(dir.join("invoicedesk.db"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let path = database_path(app.handle())?;
            let connection = db::open(&path)?;
            // Seeded only into an empty rule table, so a user's edits are
            // never undone by an upgrade - see `db::seed_rules_if_empty`.
            db::seed_rules_if_empty(&connection)?;
            app.manage(AppState {
                db: std::sync::Mutex::new(connection),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 导入
            commands::ingest::import_files,
            // 发票
            commands::invoices::list_invoices,
            commands::invoices::invoice_totals,
            commands::invoices::get_invoice,
            commands::invoices::save_invoice,
            commands::invoices::delete_invoice,
            commands::invoices::set_invoice_category,
            commands::invoices::reclassify_all,
            commands::invoices::suggest_category,
            commands::invoices::rescan_invoice,
            commands::invoices::open_source_file,
            // 规则
            commands::rules::list_rules,
            commands::rules::save_rule,
            commands::rules::delete_rule,
            commands::rules::list_categories,
            commands::rules::restore_default_rules,
            commands::rules::export_rules,
            commands::rules::import_rules,
            commands::rules::export_rules_to_file,
            commands::rules::import_rules_from_file,
            // 报销单
            commands::reports::list_reports,
            commands::reports::save_report,
            commands::reports::delete_report,
            commands::reports::set_report_invoices,
            commands::reports::report_invoices,
            commands::reports::preview_report,
            commands::reports::export_report,
            commands::reports::export_report_with_template,
            commands::reports::template_placeholders,
            // 设置与 AI
            commands::settings::get_ai_settings,
            commands::settings::set_ai_settings,
            commands::settings::get_preference,
            commands::settings::set_preference,
            commands::settings::test_ai_connection,
            commands::settings::list_provider_models,
            ai::catalog::list_providers,
            auth::keys::set_provider_api_key,
            auth::keys::provider_api_key_status,
            auth::keys::clear_provider_api_key,
            auth::custom_endpoint::set_custom_endpoint,
            auth::custom_endpoint::custom_endpoint_status,
            auth::custom_endpoint::clear_custom_endpoint,
            auth::custom_endpoint::fetch_custom_models,
            auth::custom_endpoint::test_custom_endpoint,
        ])
        .run(tauri::generate_context!())
        .expect("error while running InvoiceDesk");
}
