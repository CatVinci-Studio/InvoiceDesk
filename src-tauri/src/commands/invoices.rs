//! Reading, correcting and re-categorising stored invoices.

use crate::ai;
use crate::classify::{self, Outcome, Suggestion};
use crate::commands::AppState;
use crate::db::{self, CategoryTotal, Filter, InvoiceRow};
use crate::model::Invoice;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn list_invoices(
    state: State<'_, AppState>,
    filter: Filter,
) -> Result<Vec<InvoiceRow>, String> {
    db::list(&state.lock(), &filter)
}

#[tauri::command]
pub fn invoice_totals(
    state: State<'_, AppState>,
    filter: Filter,
) -> Result<Vec<CategoryTotal>, String> {
    db::totals_by_category(&state.lock(), &filter)
}

#[tauri::command]
pub fn get_invoice(state: State<'_, AppState>, id: i64) -> Result<Option<Invoice>, String> {
    db::load(&state.lock(), id)
}

/// Saves a user's corrections.
///
/// The frontend marks every field it edited as [`FieldSource::Manual`]
/// (confidence 1.0), which is what stops a later re-scan from overwriting the
/// correction - see `Field::merge_from`. `reviewed` is set here rather than
/// inferred, because opening a row and agreeing with it is also a review.
#[tauri::command]
pub fn save_invoice(
    state: State<'_, AppState>,
    id: i64,
    mut invoice: Invoice,
    reviewed: bool,
) -> Result<(), String> {
    // Corrections change the numbers, so the checks have to run again -
    // otherwise a fixed 价税合计 keeps its stale "对不上" warning forever.
    crate::parse::validate::infer_missing(&mut invoice);
    invoice.issues = crate::parse::validate::check(&invoice, &Default::default());
    db::update(&state.lock(), id, &invoice, reviewed)
}

#[tauri::command]
pub fn delete_invoice(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::delete(&state.lock(), id)
}

/// Sets a category by hand. Clears the rule note, because the reason is now
/// "the user said so" and attributing it to a rule would be a lie.
#[tauri::command]
pub fn set_invoice_category(
    state: State<'_, AppState>,
    id: i64,
    category: String,
) -> Result<(), String> {
    let mut invoice = db::load(&state.lock(), id)?.ok_or("发票不存在")?;
    invoice.category = Some(category);
    invoice.category_rule = None;
    let reviewed = true;
    db::update(&state.lock(), id, &invoice, reviewed)
}

/// Re-runs the rule set over stored invoices.
///
/// Called after the rule editor changes something - the whole point of
/// editing a rule is that it applies to what is already imported, not only to
/// the next import. Manually-set categories are left alone: a rule must not
/// undo a decision the user made by hand.
#[tauri::command]
pub fn reclassify_all(state: State<'_, AppState>) -> Result<usize, String> {
    let connection = state.lock();
    let rules = db::load_rules(&connection)?;
    let rows = db::list(&connection, &Filter::default())?;

    let mut changed = 0;
    for row in rows {
        // A category with no rule behind it was set by hand.
        if !row.category.is_empty() && row.category_rule.is_empty() {
            continue;
        }
        let Some(mut invoice) = db::load(&connection, row.id)? else {
            continue;
        };
        let (category, rule) = match classify::classify(&invoice, &rules) {
            Outcome::Matched { category, rule } => (Some(category), Some(rule)),
            Outcome::Unmatched => (None, None),
        };
        if invoice.category != category {
            invoice.category = category;
            invoice.category_rule = rule;
            db::update(&connection, row.id, &invoice, row.reviewed)?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Asks the model which category an invoice belongs to.
///
/// Returns a suggestion, never an applied change - accepting it is a separate
/// action, and accepting the rule that comes with it is what stops the next
/// invoice from the same vendor costing another call.
#[tauri::command]
pub async fn suggest_category(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<Suggestion, String> {
    let settings = crate::commands::settings::load_ai_settings(&state);
    if !settings.suggest_enabled {
        return Err("AI 分类建议未开启，可在「设置 → AI 识别」中打开。".to_string());
    }

    let (invoice, categories) = {
        let connection = state.lock();
        let invoice = db::load(&connection, id)?.ok_or("发票不存在")?;
        let mut categories: Vec<String> = db::load_rules(&connection)?
            .into_iter()
            .map(|r| r.category)
            .collect();
        categories.sort();
        categories.dedup();
        if !categories.iter().any(|c| c == classify::UNCATEGORISED) {
            categories.push(classify::UNCATEGORISED.to_string());
        }
        (invoice, categories)
    };

    let endpoint = ai::route::resolve(&app, &settings, ai::route::Purpose::Text)?;
    ai::categorize::suggest(&endpoint, &invoice, &categories).await
}

/// Re-reads a stored invoice's original file.
///
/// Useful after turning the vision fallback on: the rows that failed while it
/// was off can be retried without re-importing, and without losing whatever
/// the user has already corrected by hand - manual fields outrank every
/// machine source, so a re-scan cannot undo them.
#[tauri::command]
pub async fn rescan_invoice(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<Invoice, String> {
    let existing = db::load(&state.lock(), id)?.ok_or("发票不存在")?;
    let path = std::path::PathBuf::from(&existing.source_path);
    let bytes =
        std::fs::read(&path).map_err(|_| format!("找不到原始文件：{}", existing.source_path))?;

    let mut outcome = crate::extract::extract(&bytes, &path);

    let settings = crate::commands::settings::load_ai_settings(&state);
    if let Some(image) = outcome.vision_image.take() {
        if settings.vision_enabled {
            let endpoint = ai::route::resolve(&app, &settings, ai::route::Purpose::Vision)?;
            ai::vision::read_invoice(&endpoint, &image, &mut outcome.invoice).await?;
        }
    }

    // Fold the fresh reading UNDER what is already stored, so a correction
    // the user typed survives and only the gaps get filled.
    let mut merged = existing;
    merged.number.merge_from(outcome.invoice.number);
    merged.code.merge_from(outcome.invoice.code);
    merged.issued_on.merge_from(outcome.invoice.issued_on);
    merged.check_code.merge_from(outcome.invoice.check_code);
    merged.buyer_name.merge_from(outcome.invoice.buyer_name);
    merged.buyer_tax_id.merge_from(outcome.invoice.buyer_tax_id);
    merged.seller_name.merge_from(outcome.invoice.seller_name);
    merged
        .seller_tax_id
        .merge_from(outcome.invoice.seller_tax_id);
    merged
        .amount_excl_tax
        .merge_from(outcome.invoice.amount_excl_tax);
    merged.tax.merge_from(outcome.invoice.tax);
    merged.total.merge_from(outcome.invoice.total);
    merged.remark.merge_from(outcome.invoice.remark);
    if merged.items.is_empty() {
        merged.items = outcome.invoice.items;
    }
    if merged.kind.is_none() {
        merged.kind = outcome.invoice.kind;
    }

    crate::parse::validate::infer_missing(&mut merged);
    merged.issues = crate::parse::validate::check(&merged, &Default::default());

    db::update(&state.lock(), id, &merged, false)?;
    merged.id = Some(id);
    Ok(merged)
}

/// Opens the original file in whatever the OS uses for it - the fastest way
/// to answer "is that really what the invoice says?".
#[tauri::command]
pub fn open_source_file(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    // A path inside a zip (`archive.zip!entry.pdf`) has no file to open.
    if path.contains('!') {
        return Err("这张发票来自压缩包内部，没有可直接打开的文件。".to_string());
    }
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| format!("无法打开文件：{e}"))
}
