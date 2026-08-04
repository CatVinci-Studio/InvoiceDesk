//! Reimbursement sheets: assembling them, and writing them out.

use crate::commands::AppState;
use crate::db::{self, Report};
use crate::report::{self, template, ReportMeta};
use tauri::State;

#[tauri::command]
pub fn list_reports(state: State<'_, AppState>) -> Result<Vec<Report>, String> {
    db::list_reports(&state.lock())
}

#[tauri::command]
pub fn save_report(state: State<'_, AppState>, report: Report) -> Result<i64, String> {
    db::save_report(&state.lock(), &report)
}

#[tauri::command]
pub fn delete_report(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::delete_report(&state.lock(), id)
}

#[tauri::command]
pub fn set_report_invoices(
    state: State<'_, AppState>,
    report_id: i64,
    invoice_ids: Vec<i64>,
) -> Result<(), String> {
    db::set_report_invoices(&state.lock(), report_id, &invoice_ids)
}

#[tauri::command]
pub fn report_invoices(
    state: State<'_, AppState>,
    report_id: i64,
) -> Result<Vec<crate::model::Invoice>, String> {
    db::report_invoices(&state.lock(), report_id)
}

/// What a report will look like before it is written: the totals, and every
/// problem that would ride into the sheet.
///
/// Shown as a confirmation step rather than a silent export, because
/// "价税合计对不上" is exactly the kind of thing that should be settled
/// before finance sees it, not after.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPreview {
    pub count: usize,
    pub amount_excl_tax_cents: i64,
    pub tax_cents: i64,
    pub total_cents: i64,
    pub deductible_tax_cents: i64,
    pub capital_amount: String,
    pub by_category: Vec<CategoryLine>,
    /// `(发票号码, 问题)` for every invoice with something wrong.
    pub warnings: Vec<(String, String)>,
    /// Invoices in this report that share a number with another invoice in
    /// the ledger - the "已经报过了吗" check, run once more at the last
    /// moment before the sheet leaves the app.
    pub duplicates: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryLine {
    pub category: String,
    pub count: usize,
    pub total_cents: i64,
}

#[tauri::command]
pub fn preview_report(state: State<'_, AppState>, report_id: i64) -> Result<ReportPreview, String> {
    let connection = state.lock();
    let invoices = db::report_invoices(&connection, report_id)?;
    let summary = report::Summary::of(&invoices);

    let warnings = invoices
        .iter()
        .flat_map(|invoice| {
            let number = invoice
                .number
                .as_ref()
                .cloned()
                .unwrap_or_else(|| "(无号码)".to_string());
            invoice
                .issues
                .iter()
                .map(move |issue| (number.clone(), issue.message()))
        })
        .collect();

    // Checked against the WHOLE ledger, not just this report: the point is to
    // catch an invoice already claimed on last month's sheet.
    let all = db::list(&connection, &db::Filter::default())?;
    let duplicates = invoices
        .iter()
        .filter_map(|invoice| {
            let id = invoice.id?;
            let row = all.iter().find(|r| r.id == id)?;
            (!row.duplicate_of.is_empty()).then(|| row.number.clone())
        })
        .collect();

    Ok(ReportPreview {
        count: summary.count,
        amount_excl_tax_cents: summary.amount_excl_tax.cents(),
        tax_cents: summary.tax.cents(),
        total_cents: summary.total.cents(),
        deductible_tax_cents: summary.deductible_tax.cents(),
        capital_amount: report::chinese_capital(summary.total),
        by_category: summary
            .by_category
            .into_iter()
            .map(|line| CategoryLine {
                category: line.category,
                count: line.count,
                total_cents: line.total.cents(),
            })
            .collect(),
        warnings,
        duplicates,
    })
}

/// Writes the built-in workbook.
#[tauri::command]
pub fn export_report(
    state: State<'_, AppState>,
    report_id: i64,
    meta: ReportMeta,
    out_path: String,
) -> Result<(), String> {
    let invoices = db::report_invoices(&state.lock(), report_id)?;
    report::workbook(&invoices, &meta, std::path::Path::new(&out_path))
}

/// Fills the company's own template.
#[tauri::command]
pub fn export_report_with_template(
    state: State<'_, AppState>,
    report_id: i64,
    meta: ReportMeta,
    template_path: String,
    out_path: String,
) -> Result<template::FillReport, String> {
    let invoices = db::report_invoices(&state.lock(), report_id)?;
    let bytes = std::fs::read(&template_path).map_err(|e| format!("无法读取模板：{e}"))?;
    template::fill(&bytes, &invoices, &meta, std::path::Path::new(&out_path))
}

/// The placeholder reference shown next to the template picker.
#[tauri::command]
pub fn template_placeholders() -> Vec<(String, String)> {
    template::placeholders()
        .into_iter()
        .map(|(token, meaning)| (token.to_string(), meaning.to_string()))
        .collect()
}
