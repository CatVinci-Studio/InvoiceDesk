//! The local invoice ledger.
//!
//! Everything lives in one SQLite file in the app's data directory. Nothing
//! is uploaded, synced or phoned home - invoices are financial records with
//! the buyer's tax ID on them, and the only defensible default for that is
//! that they stay on the machine that read them.
//!
//! ## Why the schema is columns *plus* a JSON payload
//!
//! An [`Invoice`] has eleven [`Field`](crate::model::Field)s, each carrying a
//! value, a confidence and a source. Modelled strictly that is thirty-three
//! columns, and every future field is a migration. So instead: the whole
//! invoice is stored as JSON in `payload`, and the handful of things the app
//! actually *queries* - the number, the date, the total, the category - are
//! also projected into real, indexed columns.
//!
//! The projection is what makes filtering, sorting and duplicate detection
//! ordinary SQL, and the payload is what lets the model grow without a
//! migration. The two can only disagree if something writes a row without
//! going through [`save`], which nothing does.
//!
//! ## Duplicate detection: the actual point of keeping a ledger
//!
//! Two different things get called "duplicate", and they are handled
//! differently on purpose:
//!
//! - **The same FILE imported twice.** Caught by a unique index on the
//!   SHA-256 of the bytes. This is a no-op, not a finding - the user dropped
//!   the same folder in twice, and the right response is to say so quietly.
//! - **The same INVOICE seen twice.** Caught by 发票代码+发票号码 across
//!   different files. This is the finding: it means the invoice may already
//!   have been claimed, possibly in a different month, possibly by a
//!   different person. It is never blocked - the user has to be able to see
//!   both rows to decide - it is flagged.

use crate::classify::Rule;
use crate::model::{Invoice, Money};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Bumped whenever `migrate` gains a step.
const SCHEMA_VERSION: i64 = 1;

pub fn open(path: &std::path::Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建数据目录：{e}"))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("无法打开数据库：{e}"))?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// An in-memory ledger, for tests.
pub fn open_in_memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        // WAL so a long import does not block the list query behind it, and
        // foreign keys so deleting a report cannot orphan its lines.
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;",
    )
    .map_err(|e| format!("数据库初始化失败：{e}"))
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;

    if version < 1 {
        conn.execute_batch(
            r#"
            CREATE TABLE invoices (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                -- Projected from payload for querying; see the module docs.
                number         TEXT    NOT NULL DEFAULT '',
                code           TEXT    NOT NULL DEFAULT '',
                -- 发票号码 alone is unique only on 数电票; older invoices are
                -- identified by 代码+号码 together, so the dedup key is both.
                dedup_key      TEXT    NOT NULL DEFAULT '',
                issued_on      TEXT    NOT NULL DEFAULT '',
                kind           TEXT    NOT NULL DEFAULT '',
                seller_name    TEXT    NOT NULL DEFAULT '',
                buyer_name     TEXT    NOT NULL DEFAULT '',
                total_cents    INTEGER NOT NULL DEFAULT 0,
                tax_cents      INTEGER NOT NULL DEFAULT 0,
                category       TEXT    NOT NULL DEFAULT '',
                category_rule  TEXT    NOT NULL DEFAULT '',
                min_confidence REAL    NOT NULL DEFAULT 0,
                issue_count    INTEGER NOT NULL DEFAULT 0,
                -- Set once the user has looked at the detail pane and
                -- confirmed the fields. Survives re-classification.
                reviewed       INTEGER NOT NULL DEFAULT 0,
                file_hash      TEXT    NOT NULL,
                source_path    TEXT    NOT NULL DEFAULT '',
                payload        TEXT    NOT NULL,
                imported_at    TEXT    NOT NULL
            );

            -- The same file twice is a no-op, so it is enforced here rather
            -- than checked in application code.
            CREATE UNIQUE INDEX invoices_file_hash ON invoices(file_hash);
            -- The same invoice twice is a FINDING, so this index is
            -- deliberately NOT unique - both rows have to be visible.
            CREATE INDEX invoices_dedup_key ON invoices(dedup_key)
                WHERE dedup_key <> '';
            CREATE INDEX invoices_issued_on ON invoices(issued_on);
            CREATE INDEX invoices_category ON invoices(category);

            CREATE TABLE rules (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT    NOT NULL,
                category   TEXT    NOT NULL,
                priority   INTEGER NOT NULL DEFAULT 100,
                enabled    INTEGER NOT NULL DEFAULT 1,
                conditions TEXT    NOT NULL
            );

            CREATE TABLE reports (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                title      TEXT NOT NULL,
                applicant  TEXT NOT NULL DEFAULT '',
                department TEXT NOT NULL DEFAULT '',
                note       TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );

            CREATE TABLE report_invoices (
                report_id  INTEGER NOT NULL REFERENCES reports(id) ON DELETE CASCADE,
                invoice_id INTEGER NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
                position   INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (report_id, invoice_id)
            );

            CREATE TABLE settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| format!("建表失败：{e}"))?;
    }

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Invoices
// ---------------------------------------------------------------------------

/// The key two files must share to be the same invoice.
///
/// Empty when there is no 发票号码 at all, and an empty key never matches
/// anything - an unreadable scan must not be reported as a duplicate of
/// every other unreadable scan.
pub fn dedup_key(invoice: &Invoice) -> String {
    match invoice.number.as_ref() {
        Some(number) if !number.is_empty() => {
            let code = invoice.code.as_ref().map(String::as_str).unwrap_or("");
            format!("{code}-{number}")
        }
        _ => String::new(),
    }
}

/// What [`save`] did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SaveOutcome {
    Inserted {
        id: i64,
    },
    /// The exact same file is already in the ledger; nothing changed.
    AlreadyImported {
        id: i64,
    },
    Updated {
        id: i64,
    },
}

impl SaveOutcome {
    pub fn id(&self) -> i64 {
        match self {
            SaveOutcome::Inserted { id }
            | SaveOutcome::AlreadyImported { id }
            | SaveOutcome::Updated { id } => *id,
        }
    }
}

/// Inserts an invoice, or reports that its file was already imported.
pub fn save(conn: &Connection, invoice: &Invoice) -> Result<SaveOutcome, String> {
    if let Some(existing) = id_by_file_hash(conn, &invoice.file_hash)? {
        return Ok(SaveOutcome::AlreadyImported { id: existing });
    }

    let payload = serde_json::to_string(invoice).map_err(|e| e.to_string())?;
    conn.execute(
        r#"INSERT INTO invoices
             (number, code, dedup_key, issued_on, kind, seller_name, buyer_name,
              total_cents, tax_cents, category, category_rule, min_confidence,
              issue_count, reviewed, file_hash, source_path, payload, imported_at)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0,?14,?15,?16,?17)"#,
        params![
            invoice.number.as_ref().map(String::as_str).unwrap_or(""),
            invoice.code.as_ref().map(String::as_str).unwrap_or(""),
            dedup_key(invoice),
            invoice.issued_on.as_ref().map(String::as_str).unwrap_or(""),
            invoice.kind.map(|k| k.label()).unwrap_or(""),
            invoice
                .seller_name
                .as_ref()
                .map(String::as_str)
                .unwrap_or(""),
            invoice
                .buyer_name
                .as_ref()
                .map(String::as_str)
                .unwrap_or(""),
            invoice.total.value.unwrap_or(Money::ZERO).cents(),
            invoice.tax.value.unwrap_or(Money::ZERO).cents(),
            invoice.category.as_deref().unwrap_or(""),
            invoice.category_rule.as_deref().unwrap_or(""),
            invoice.min_key_confidence(),
            invoice.issues.len() as i64,
            invoice.file_hash,
            invoice.source_path,
            payload,
            chrono::Local::now().to_rfc3339(),
        ],
    )
    .map_err(|e| format!("写入发票失败：{e}"))?;

    Ok(SaveOutcome::Inserted {
        id: conn.last_insert_rowid(),
    })
}

/// Overwrites an existing invoice - used after the user edits fields in the
/// review pane, which is also when `reviewed` gets set.
pub fn update(conn: &Connection, id: i64, invoice: &Invoice, reviewed: bool) -> Result<(), String> {
    let payload = serde_json::to_string(invoice).map_err(|e| e.to_string())?;
    conn.execute(
        r#"UPDATE invoices SET
             number=?1, code=?2, dedup_key=?3, issued_on=?4, kind=?5,
             seller_name=?6, buyer_name=?7, total_cents=?8, tax_cents=?9,
             category=?10, category_rule=?11, min_confidence=?12,
             issue_count=?13, reviewed=?14, payload=?15
           WHERE id=?16"#,
        params![
            invoice.number.as_ref().map(String::as_str).unwrap_or(""),
            invoice.code.as_ref().map(String::as_str).unwrap_or(""),
            dedup_key(invoice),
            invoice.issued_on.as_ref().map(String::as_str).unwrap_or(""),
            invoice.kind.map(|k| k.label()).unwrap_or(""),
            invoice
                .seller_name
                .as_ref()
                .map(String::as_str)
                .unwrap_or(""),
            invoice
                .buyer_name
                .as_ref()
                .map(String::as_str)
                .unwrap_or(""),
            invoice.total.value.unwrap_or(Money::ZERO).cents(),
            invoice.tax.value.unwrap_or(Money::ZERO).cents(),
            invoice.category.as_deref().unwrap_or(""),
            invoice.category_rule.as_deref().unwrap_or(""),
            invoice.min_key_confidence(),
            invoice.issues.len() as i64,
            reviewed as i64,
            payload,
            id,
        ],
    )
    .map_err(|e| format!("更新发票失败：{e}"))?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM invoices WHERE id=?1", params![id])
        .map_err(|e| format!("删除失败：{e}"))?;
    Ok(())
}

fn id_by_file_hash(conn: &Connection, hash: &str) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT id FROM invoices WHERE file_hash=?1",
        params![hash],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn load(conn: &Connection, id: i64) -> Result<Option<Invoice>, String> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload FROM invoices WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    payload
        .map(|p| {
            serde_json::from_str::<Invoice>(&p)
                .map(|mut inv| {
                    inv.id = Some(id);
                    inv
                })
                .map_err(|e| format!("发票数据损坏：{e}"))
        })
        .transpose()
}

/// A row of the invoice list - everything the table shows, and nothing else.
/// The full invoice is fetched only when a row is opened.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceRow {
    pub id: i64,
    pub number: String,
    pub issued_on: String,
    pub kind: String,
    pub seller_name: String,
    pub total_cents: i64,
    pub tax_cents: i64,
    pub category: String,
    pub category_rule: String,
    pub min_confidence: f32,
    pub issue_count: i64,
    pub reviewed: bool,
    pub source_path: String,
    /// Ids of other rows carrying the same 发票代码+号码. Non-empty means
    /// "possibly already claimed" - see the module docs.
    pub duplicate_of: Vec<i64>,
}

/// What the list is filtered by. Every field is optional; an absent one means
/// no constraint on that axis.
///
/// `#[serde(default)]` on the struct is load-bearing, not decorative. Without
/// it serde requires the two `bool` fields to be present in the JSON, so a
/// caller asking the honest question "give me everything" - `list({})` - gets
/// `missing field 'needsReviewOnly'` instead of an answer, and every call
/// site has to spell out fields it does not care about. With it, a partial
/// filter means exactly what it reads as.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    /// Matches 发票号码, 销售方名称 or 备注, case-insensitively.
    pub search: Option<String>,
    pub category: Option<String>,
    /// Inclusive `YYYY-MM-DD` bounds.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Only rows with unreviewed low-confidence fields or validation issues.
    pub needs_review_only: bool,
    /// Only rows that share an invoice number with another row.
    pub duplicates_only: bool,
    /// Exclude rows already attached to this report.
    pub exclude_report: Option<i64>,
}

pub fn list(conn: &Connection, filter: &Filter) -> Result<Vec<InvoiceRow>, String> {
    let mut sql = String::from(
        r#"SELECT i.id, i.number, i.issued_on, i.kind, i.seller_name, i.total_cents,
                  i.tax_cents, i.category, i.category_rule, i.min_confidence,
                  i.issue_count, i.reviewed, i.source_path,
                  (SELECT group_concat(d.id) FROM invoices d
                    WHERE d.dedup_key = i.dedup_key AND d.dedup_key <> '' AND d.id <> i.id)
           FROM invoices i WHERE 1=1"#,
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(search) = filter.search.as_ref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" AND (i.number LIKE ?  OR i.seller_name LIKE ? OR i.buyer_name LIKE ?)");
        let pattern = format!("%{}%", search.trim());
        for _ in 0..3 {
            args.push(Box::new(pattern.clone()));
        }
    }
    if let Some(category) = &filter.category {
        sql.push_str(" AND i.category = ?");
        args.push(Box::new(category.clone()));
    }
    if let Some(from) = &filter.from {
        sql.push_str(" AND i.issued_on >= ?");
        args.push(Box::new(from.clone()));
    }
    if let Some(to) = &filter.to {
        sql.push_str(" AND i.issued_on <= ?");
        args.push(Box::new(to.clone()));
    }
    if filter.needs_review_only {
        // Unreviewed AND actually questionable - a clean XML-sourced invoice
        // nobody has opened is not something to nag about.
        sql.push_str(" AND i.reviewed = 0 AND (i.min_confidence < 0.9 OR i.issue_count > 0)");
    }
    if filter.duplicates_only {
        sql.push_str(
            " AND i.dedup_key <> '' AND EXISTS (SELECT 1 FROM invoices d
                WHERE d.dedup_key = i.dedup_key AND d.id <> i.id)",
        );
    }
    if let Some(report) = filter.exclude_report {
        sql.push_str(
            " AND i.id NOT IN (SELECT invoice_id FROM report_invoices WHERE report_id = ?)",
        );
        args.push(Box::new(report));
    }

    // Newest invoice first; the id tiebreak keeps same-day rows in import
    // order rather than an arbitrary one.
    sql.push_str(" ORDER BY i.issued_on DESC, i.id DESC");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params = rusqlite::params_from_iter(args.iter().map(|b| b.as_ref()));
    let rows = stmt
        .query_map(params, |r| {
            let duplicates: Option<String> = r.get(13)?;
            Ok(InvoiceRow {
                id: r.get(0)?,
                number: r.get(1)?,
                issued_on: r.get(2)?,
                kind: r.get(3)?,
                seller_name: r.get(4)?,
                total_cents: r.get(5)?,
                tax_cents: r.get(6)?,
                category: r.get(7)?,
                category_rule: r.get(8)?,
                min_confidence: r.get(9)?,
                issue_count: r.get(10)?,
                reviewed: r.get::<_, i64>(11)? != 0,
                source_path: r.get(12)?,
                duplicate_of: duplicates
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|s| s.parse().ok())
                    .collect(),
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取发票列表失败：{e}"))
}

/// Per-category totals over the same filter the list uses, so the summary bar
/// and the table can never disagree about what is on screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryTotal {
    pub category: String,
    pub count: i64,
    pub total_cents: i64,
    pub tax_cents: i64,
}

pub fn totals_by_category(
    conn: &Connection,
    filter: &Filter,
) -> Result<Vec<CategoryTotal>, String> {
    // Reuses `list` rather than duplicating the WHERE clause: two copies of
    // the filter logic is exactly how a summary starts disagreeing with the
    // table under it, and an expense ledger is not large enough for the
    // round trip to matter.
    let rows = list(conn, filter)?;
    let mut totals: Vec<CategoryTotal> = Vec::new();

    for row in rows {
        let category = if row.category.is_empty() {
            crate::classify::UNCATEGORISED.to_string()
        } else {
            row.category
        };
        match totals.iter_mut().find(|t| t.category == category) {
            Some(entry) => {
                entry.count += 1;
                entry.total_cents += row.total_cents;
                entry.tax_cents += row.tax_cents;
            }
            None => totals.push(CategoryTotal {
                category,
                count: 1,
                total_cents: row.total_cents,
                tax_cents: row.tax_cents,
            }),
        }
    }

    totals.sort_by_key(|line| std::cmp::Reverse(line.total_cents));
    Ok(totals)
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

pub fn load_rules(conn: &Connection) -> Result<Vec<Rule>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, category, priority, enabled, conditions FROM rules ORDER BY priority DESC, id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let conditions: String = r.get(5)?;
            Ok(Rule {
                id: Some(r.get(0)?),
                name: r.get(1)?,
                category: r.get(2)?,
                priority: r.get(3)?,
                enabled: r.get::<_, i64>(4)? != 0,
                conditions: serde_json::from_str(&conditions).unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取规则失败：{e}"))
}

pub fn save_rule(conn: &Connection, rule: &Rule) -> Result<i64, String> {
    let conditions = serde_json::to_string(&rule.conditions).map_err(|e| e.to_string())?;
    match rule.id {
        Some(id) => {
            conn.execute(
                "UPDATE rules SET name=?1, category=?2, priority=?3, enabled=?4, conditions=?5 WHERE id=?6",
                params![rule.name, rule.category, rule.priority, rule.enabled as i64, conditions, id],
            )
            .map_err(|e| format!("保存规则失败：{e}"))?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO rules (name, category, priority, enabled, conditions) VALUES (?1,?2,?3,?4,?5)",
                params![rule.name, rule.category, rule.priority, rule.enabled as i64, conditions],
            )
            .map_err(|e| format!("保存规则失败：{e}"))?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn delete_rule(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM rules WHERE id=?1", params![id])
        .map_err(|e| format!("删除规则失败：{e}"))?;
    Ok(())
}

/// Writes the built-in rules, but only into an empty rule table.
///
/// The guard is the whole point: a user who deleted 「销售方名称：酒店」
/// because it kept mis-filing their client dinners must not find it back
/// after an update.
pub fn seed_rules_if_empty(conn: &Connection) -> Result<usize, String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM rules", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if count > 0 {
        return Ok(0);
    }
    let seeds = crate::classify::defaults::rules();
    for rule in &seeds {
        save_rule(conn, rule)?;
    }
    Ok(seeds.len())
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: Option<i64>,
    pub title: String,
    pub applicant: String,
    pub department: String,
    pub note: String,
    #[serde(default)]
    pub created_at: String,
    /// Filled by [`list_reports`]; ignored on save.
    #[serde(default)]
    pub invoice_count: i64,
    #[serde(default)]
    pub total_cents: i64,
}

pub fn save_report(conn: &Connection, report: &Report) -> Result<i64, String> {
    match report.id {
        Some(id) => {
            conn.execute(
                "UPDATE reports SET title=?1, applicant=?2, department=?3, note=?4 WHERE id=?5",
                params![
                    report.title,
                    report.applicant,
                    report.department,
                    report.note,
                    id
                ],
            )
            .map_err(|e| format!("保存报销单失败：{e}"))?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO reports (title, applicant, department, note, created_at) VALUES (?1,?2,?3,?4,?5)",
                params![
                    report.title,
                    report.applicant,
                    report.department,
                    report.note,
                    chrono::Local::now().to_rfc3339()
                ],
            )
            .map_err(|e| format!("保存报销单失败：{e}"))?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn list_reports(conn: &Connection) -> Result<Vec<Report>, String> {
    let mut stmt = conn
        .prepare(
            r#"SELECT r.id, r.title, r.applicant, r.department, r.note, r.created_at,
                      COUNT(ri.invoice_id), COALESCE(SUM(i.total_cents), 0)
               FROM reports r
               LEFT JOIN report_invoices ri ON ri.report_id = r.id
               LEFT JOIN invoices i ON i.id = ri.invoice_id
               GROUP BY r.id ORDER BY r.created_at DESC"#,
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Report {
                id: Some(r.get(0)?),
                title: r.get(1)?,
                applicant: r.get(2)?,
                department: r.get(3)?,
                note: r.get(4)?,
                created_at: r.get(5)?,
                invoice_count: r.get(6)?,
                total_cents: r.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取报销单失败：{e}"))
}

pub fn delete_report(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM reports WHERE id=?1", params![id])
        .map_err(|e| format!("删除报销单失败：{e}"))?;
    Ok(())
}

pub fn set_report_invoices(conn: &Connection, report_id: i64, ids: &[i64]) -> Result<(), String> {
    conn.execute(
        "DELETE FROM report_invoices WHERE report_id=?1",
        params![report_id],
    )
    .map_err(|e| e.to_string())?;
    for (position, invoice_id) in ids.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO report_invoices (report_id, invoice_id, position) VALUES (?1,?2,?3)",
            params![report_id, invoice_id, position as i64],
        )
        .map_err(|e| format!("加入报销单失败：{e}"))?;
    }
    Ok(())
}

/// The invoices on a report, in the order they were added.
pub fn report_invoices(conn: &Connection, report_id: i64) -> Result<Vec<Invoice>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.payload FROM report_invoices ri
             JOIN invoices i ON i.id = ri.invoice_id
             WHERE ri.report_id = ?1 ORDER BY ri.position, i.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![report_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows {
        let (id, payload) = row.map_err(|e| e.to_string())?;
        let mut invoice: Invoice =
            serde_json::from_str(&payload).map_err(|e| format!("发票数据损坏：{e}"))?;
        invoice.id = Some(id);
        out.push(invoice);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key=?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| format!("保存设置失败：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, FieldSource, InvoiceKind};

    fn invoice(number: &str, code: &str, hash: &str, total: i64) -> Invoice {
        Invoice {
            kind: Some(InvoiceKind::DigitalGeneral),
            number: Field::new(number.to_string(), FieldSource::Xml),
            code: if code.is_empty() {
                Field::default()
            } else {
                Field::new(code.to_string(), FieldSource::Xml)
            },
            issued_on: Field::new("2024-03-01".to_string(), FieldSource::Xml),
            seller_name: Field::new("某某公司".to_string(), FieldSource::Xml),
            total: Field::new(Money(total), FieldSource::Xml),
            file_hash: hash.to_string(),
            category: Some("住宿".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn the_same_file_twice_is_a_no_op() {
        let conn = open_in_memory().unwrap();
        let inv = invoice("24312000000012345678", "", "hash-a", 106_000);

        assert!(matches!(
            save(&conn, &inv).unwrap(),
            SaveOutcome::Inserted { .. }
        ));
        assert!(matches!(
            save(&conn, &inv).unwrap(),
            SaveOutcome::AlreadyImported { .. }
        ));
        assert_eq!(list(&conn, &Filter::default()).unwrap().len(), 1);
    }

    /// The feature this whole ledger exists for: the same invoice arriving as
    /// two different files must produce two visible rows that point at each
    /// other, not one row and not an error.
    #[test]
    fn the_same_invoice_from_two_files_is_flagged_not_blocked() {
        let conn = open_in_memory().unwrap();
        save(
            &conn,
            &invoice("24312000000012345678", "", "hash-a", 106_000),
        )
        .unwrap();
        save(
            &conn,
            &invoice("24312000000012345678", "", "hash-b", 106_000),
        )
        .unwrap();

        let rows = list(&conn, &Filter::default()).unwrap();
        assert_eq!(rows.len(), 2, "两行都必须可见");
        assert!(rows.iter().all(|r| !r.duplicate_of.is_empty()));

        let flagged = list(
            &conn,
            &Filter {
                duplicates_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(flagged.len(), 2);
    }

    /// Old invoices are identified by 代码+号码 together. Two invoices from
    /// different issuers can share a 发票号码 legitimately, and calling them
    /// duplicates would cry wolf on every import.
    #[test]
    fn the_same_number_under_different_codes_is_not_a_duplicate() {
        let conn = open_in_memory().unwrap();
        save(&conn, &invoice("12345678", "3100152130", "hash-a", 10_000)).unwrap();
        save(&conn, &invoice("12345678", "3100152131", "hash-b", 20_000)).unwrap();

        let rows = list(&conn, &Filter::default()).unwrap();
        assert!(rows.iter().all(|r| r.duplicate_of.is_empty()));
    }

    /// An unreadable scan has no number. Those must not all be reported as
    /// duplicates of each other.
    #[test]
    fn invoices_with_no_number_are_never_duplicates() {
        let conn = open_in_memory().unwrap();
        for hash in ["a", "b", "c"] {
            let mut blank = invoice("", "", hash, 0);
            blank.number = Field::default();
            save(&conn, &blank).unwrap();
        }
        let rows = list(&conn, &Filter::default()).unwrap();
        assert!(rows.iter().all(|r| r.duplicate_of.is_empty()));
    }

    #[test]
    fn round_trips_the_full_invoice_through_the_payload() {
        let conn = open_in_memory().unwrap();
        let mut original = invoice("24312000000012345678", "", "hash-a", 106_000);
        original.items = vec![crate::model::InvoiceItem {
            name: "*住宿服务*住宿费".to_string(),
            amount: Some(Money(100_000)),
            ..Default::default()
        }];
        let id = save(&conn, &original).unwrap().id();

        let loaded = load(&conn, id).unwrap().expect("存在");
        assert_eq!(loaded.id, Some(id));
        assert_eq!(loaded.items.len(), 1);
        // Provenance survives the round trip - that is what the payload is for.
        assert_eq!(loaded.total.source, FieldSource::Xml);
        assert_eq!(loaded.total.confidence, 1.0);
    }

    #[test]
    fn filters_narrow_the_list() {
        let conn = open_in_memory().unwrap();
        let mut a = invoice("24312000000012345678", "", "h1", 106_000);
        a.issued_on = Field::new("2024-03-01".into(), FieldSource::Xml);
        let mut b = invoice("24312000000087654321", "", "h2", 50_000);
        b.issued_on = Field::new("2024-05-20".into(), FieldSource::Xml);
        b.category = Some("餐饮".into());
        save(&conn, &a).unwrap();
        save(&conn, &b).unwrap();

        let march = list(
            &conn,
            &Filter {
                from: Some("2024-03-01".into()),
                to: Some("2024-03-31".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(march.len(), 1);

        let food = list(
            &conn,
            &Filter {
                category: Some("餐饮".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(food.len(), 1);
        assert_eq!(food[0].total_cents, 50_000);
    }

    /// The summary bar and the table must never disagree about what is on
    /// screen, so they run through the same filter.
    #[test]
    fn totals_match_the_filtered_list() {
        let conn = open_in_memory().unwrap();
        save(&conn, &invoice("24312000000012345678", "", "h1", 106_000)).unwrap();
        let mut other = invoice("24312000000087654321", "", "h2", 50_000);
        other.category = Some("餐饮".into());
        save(&conn, &other).unwrap();

        let totals = totals_by_category(&conn, &Filter::default()).unwrap();
        assert_eq!(totals.len(), 2);
        assert_eq!(totals[0].category, "住宿", "按金额倒序");
        assert_eq!(totals.iter().map(|t| t.total_cents).sum::<i64>(), 156_000);
    }

    #[test]
    fn seeding_never_overwrites_a_users_edited_rule_set() {
        let conn = open_in_memory().unwrap();
        assert!(seed_rules_if_empty(&conn).unwrap() > 0);

        let rules = load_rules(&conn).unwrap();
        delete_rule(&conn, rules[0].id.unwrap()).unwrap();
        let after_delete = load_rules(&conn).unwrap().len();

        // An "upgrade" must not resurrect it.
        assert_eq!(seed_rules_if_empty(&conn).unwrap(), 0);
        assert_eq!(load_rules(&conn).unwrap().len(), after_delete);
    }

    #[test]
    fn reports_carry_their_invoices_and_totals() {
        let conn = open_in_memory().unwrap();
        let a = save(&conn, &invoice("24312000000012345678", "", "h1", 106_000))
            .unwrap()
            .id();
        let b = save(&conn, &invoice("24312000000087654321", "", "h2", 50_000))
            .unwrap()
            .id();

        let report_id = save_report(
            &conn,
            &Report {
                id: None,
                title: "2024年3月差旅".into(),
                applicant: "沈".into(),
                department: "研发".into(),
                note: String::new(),
                created_at: String::new(),
                invoice_count: 0,
                total_cents: 0,
            },
        )
        .unwrap();
        set_report_invoices(&conn, report_id, &[b, a]).unwrap();

        let reports = list_reports(&conn).unwrap();
        assert_eq!(reports[0].invoice_count, 2);
        assert_eq!(reports[0].total_cents, 156_000);

        // Order is the order they were added, not id order.
        let invoices = report_invoices(&conn, report_id).unwrap();
        assert_eq!(invoices[0].id, Some(b));
    }

    #[test]
    fn deleting_a_report_leaves_its_invoices_alone() {
        let conn = open_in_memory().unwrap();
        let a = save(&conn, &invoice("24312000000012345678", "", "h1", 106_000))
            .unwrap()
            .id();
        let report_id = save_report(
            &conn,
            &Report {
                id: None,
                title: "t".into(),
                applicant: String::new(),
                department: String::new(),
                note: String::new(),
                created_at: String::new(),
                invoice_count: 0,
                total_cents: 0,
            },
        )
        .unwrap();
        set_report_invoices(&conn, report_id, &[a]).unwrap();

        delete_report(&conn, report_id).unwrap();
        assert!(load(&conn, a).unwrap().is_some(), "发票不应随报销单删除");
    }

    /// Regression guard for a bug that made the whole list pane fail on
    /// launch: serde requires a plain `bool` to be present, so `list({})`
    /// from the frontend came back as
    /// `missing field 'needsReviewOnly'` rather than as every invoice.
    #[test]
    fn a_partial_filter_deserialises() {
        let filter: Filter = serde_json::from_str("{}").expect("空筛选条件必须可用");
        assert!(!filter.needs_review_only);
        assert!(!filter.duplicates_only);
        assert_eq!(filter.search, None);

        let scoped: Filter =
            serde_json::from_str(r#"{"excludeReport": 3}"#).expect("部分筛选条件必须可用");
        assert_eq!(scoped.exclude_report, Some(3));
        assert!(!scoped.duplicates_only);

        let full: Filter = serde_json::from_str(
            r#"{"search":"酒店","category":"住宿","from":"2024-01-01","to":"2024-12-31",
                "needsReviewOnly":true,"duplicatesOnly":false,"excludeReport":null}"#,
        )
        .expect("完整筛选条件");
        assert_eq!(full.search.as_deref(), Some("酒店"));
        assert!(full.needs_review_only);
    }

    #[test]
    fn settings_upsert() {
        let conn = open_in_memory().unwrap();
        assert_eq!(get_setting(&conn, "provider").unwrap(), None);
        set_setting(&conn, "provider", "qwen").unwrap();
        set_setting(&conn, "provider", "zhipu").unwrap();
        assert_eq!(
            get_setting(&conn, "provider").unwrap().as_deref(),
            Some("zhipu")
        );
    }
}
