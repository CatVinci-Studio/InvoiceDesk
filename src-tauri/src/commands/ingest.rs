//! Importing files: the one command with real work behind it.
//!
//! Progress streams back over a Tauri channel rather than being polled,
//! because a folder of two hundred invoices takes long enough that a frozen
//! window is the difference between "working" and "broken". Each file
//! reports as it lands, with the trace of which extraction layer produced its
//! fields - so a user watching an import can already see *why* one invoice
//! came out complete and another needs review.

use crate::classify::{self, Outcome};
use crate::commands::AppState;
use crate::db;
use crate::extract::{self, TraceStep};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    /// Read cleanly and stored.
    Imported,
    /// Stored, but fields need a human before it can go on a report.
    NeedsReview,
    /// This exact file is already in the ledger; nothing changed.
    AlreadyImported,
    /// Nothing usable came out. Still stored, so the user can type the
    /// fields in rather than losing track of the file entirely.
    Failed,
}

/// One live fragment of an in-flight import.
///
/// Mirrored by `IngestEvent` in `src/ingest/ipc.ts` - the two must stay in
/// sync.
#[derive(Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum IngestEvent {
    Started {
        total: usize,
    },
    File {
        index: usize,
        name: String,
        status: FileStatus,
        invoice_id: Option<i64>,
        /// Which extraction layers were tried, and what each produced.
        trace: Vec<TraceStep>,
        /// Empty unless something went wrong.
        message: String,
        /// True when the offline layers fell short and the vision fallback
        /// could have helped but is switched off. Surfaced so the user is
        /// told the setting exists at the moment it would have mattered,
        /// rather than discovering it later in a settings pane.
        vision_would_help: bool,
    },
    Finished {
        imported: usize,
        needs_review: usize,
        duplicates: usize,
        failed: usize,
    },
}

/// Expands the dropped paths into the actual files to read.
///
/// Directories are walked (people drag the whole 发票 folder) and non-OFD
/// zips are unpacked in memory (people forward the mail attachment intact).
/// Both are common enough that refusing them would just move the work back
/// onto the user.
fn expand(paths: &[PathBuf]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for path in paths {
        collect(path, &mut out, 0);
    }
    out
}

fn collect(path: &Path, out: &mut Vec<(String, Vec<u8>)>, depth: usize) {
    // Bounded: a symlink loop in a dropped folder must not spin forever.
    if depth > 8 {
        return;
    }

    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        // Alphabetical, so an import of the same folder twice reports in the
        // same order and the progress list is comparable.
        children.sort();
        for child in children {
            collect(&child, out, depth + 1);
        }
        return;
    }

    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let name = path.to_string_lossy().into_owned();

    if extract::detect::detect(&bytes, path) == extract::detect::FileKind::Zip {
        expand_zip(&bytes, &name, out);
        return;
    }
    out.push((name, bytes));
}

fn expand_zip(bytes: &[u8], archive_name: &str, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return;
    };
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    for name in names {
        if name.ends_with('/') {
            continue;
        }
        let Ok(entry) = archive.by_name(&name) else {
            continue;
        };
        let mut buf = Vec::new();
        // Capped so a zip bomb cannot exhaust memory during an import.
        if entry.take(64 * 1024 * 1024).read_to_end(&mut buf).is_err() {
            continue;
        }
        // The displayed path keeps the archive in it, so a user can tell
        // which of three mailed zips a problem invoice came from.
        out.push((format!("{archive_name}!{name}"), buf));
    }
}

/// Reads one file end to end: extract, optionally ask a model, classify.
async fn process_one(
    app: &AppHandle,
    state: &AppState,
    name: &str,
    bytes: &[u8],
) -> (crate::model::Invoice, Vec<TraceStep>, bool, String) {
    let path = PathBuf::from(name);
    let mut outcome = extract::extract(bytes, &path);
    let mut message = String::new();

    let settings = crate::commands::settings::load_ai_settings(state);
    let mut vision_would_help = false;

    if let Some(image) = outcome.vision_image.take() {
        if settings.vision_enabled {
            match crate::ai::route::resolve(app, &settings, crate::ai::route::Purpose::Vision) {
                Ok(endpoint) => {
                    match crate::ai::vision::read_invoice(&endpoint, &image, &mut outcome.invoice)
                        .await
                    {
                        Ok(_) => outcome.trace.push(TraceStep {
                            layer: "AI 识别",
                            hit: true,
                            detail: format!("由 {} 补全剩余字段", endpoint.model),
                        }),
                        Err(error) => {
                            outcome.trace.push(TraceStep {
                                layer: "AI 识别",
                                hit: false,
                                detail: error.clone(),
                            });
                            message = error;
                        }
                    }
                }
                Err(error) => {
                    outcome.trace.push(TraceStep {
                        layer: "AI 识别",
                        hit: false,
                        detail: error.clone(),
                    });
                    message = error;
                }
            }

            // The model may have supplied the missing pieces; re-derive and
            // re-check so the stored issues reflect the final field set.
            crate::parse::validate::infer_missing(&mut outcome.invoice);
            outcome.invoice.issues =
                crate::parse::validate::check(&outcome.invoice, &Default::default());
        } else {
            vision_would_help = true;
            outcome.trace.push(TraceStep {
                layer: "AI 识别",
                hit: false,
                detail: "未开启，可在「设置 → AI 识别」中打开".to_string(),
            });
        }
    }

    // Rules only. A category suggestion costs an API call per invoice, which
    // is not something to spend silently on a 200-file import - it is offered
    // afterwards, on the rows that actually need it.
    let rules = db::load_rules(&state.lock()).unwrap_or_default();
    if let Outcome::Matched { category, rule } = classify::classify(&outcome.invoice, &rules) {
        outcome.invoice.category = Some(category);
        outcome.invoice.category_rule = Some(rule);
    }

    (outcome.invoice, outcome.trace, vision_would_help, message)
}

/// Imports files or folders, streaming progress to `channel`.
#[tauri::command]
pub async fn import_files(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<PathBuf>,
    channel: Channel<IngestEvent>,
) -> Result<(), String> {
    let files = expand(&paths);
    // A dropped-then-closed window must not fail the import; the ledger is
    // written either way, so send failures are ignored throughout.
    let _ = channel.send(IngestEvent::Started { total: files.len() });

    let (mut imported, mut needs_review, mut duplicates, mut failed) = (0, 0, 0, 0);

    for (index, (name, bytes)) in files.iter().enumerate() {
        let (invoice, trace, vision_would_help, message) =
            process_one(&app, &state, name, bytes).await;

        let complete = extract::is_complete(&invoice);
        let usable = invoice.number.is_present() || invoice.total.is_present();

        let saved = db::save(&state.lock(), &invoice);
        let (status, invoice_id, mut message) = match saved {
            Ok(db::SaveOutcome::AlreadyImported { id }) => {
                duplicates += 1;
                (FileStatus::AlreadyImported, Some(id), String::new())
            }
            Ok(outcome) => {
                let status = if complete {
                    imported += 1;
                    FileStatus::Imported
                } else if usable {
                    needs_review += 1;
                    FileStatus::NeedsReview
                } else {
                    failed += 1;
                    FileStatus::Failed
                };
                (status, Some(outcome.id()), message)
            }
            Err(error) => {
                failed += 1;
                (FileStatus::Failed, None, error)
            }
        };

        if status == FileStatus::Failed && message.is_empty() {
            message = "没能从这个文件里读出发票信息，可以手动补录".to_string();
        }

        let _ = channel.send(IngestEvent::File {
            index,
            name: Path::new(name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.clone()),
            status,
            invoice_id,
            trace,
            message,
            vision_would_help,
        });
    }

    let _ = channel.send(IngestEvent::Finished {
        imported,
        needs_review,
        duplicates,
        failed,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_folder_is_walked_and_its_files_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.xml"), "<b/>").unwrap();
        std::fs::write(dir.path().join("a.xml"), "<a/>").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/c.xml"), "<c/>").unwrap();

        let files = expand(&[dir.path().to_path_buf()]);
        assert_eq!(files.len(), 3);
        // Alphabetical, so re-importing the same folder reports identically.
        assert!(files[0].0.ends_with("a.xml"));
        assert!(files[1].0.ends_with("b.xml"));
    }

    #[test]
    fn a_zip_is_unpacked_rather_than_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("发票.zip");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            for name in ["one.xml", "two.xml"] {
                writer
                    .start_file::<_, ()>(name, Default::default())
                    .unwrap();
                writer.write_all(b"<x/>").unwrap();
            }
            writer.finish().unwrap();
        }

        let files = expand(&[path]);
        assert_eq!(files.len(), 2);
        // The archive stays in the displayed name, so a problem invoice can
        // be traced back to which zip it came from.
        assert!(files[0].0.contains("发票.zip!"), "{}", files[0].0);
    }

    /// An OFD is also a zip. Unpacking it would turn one invoice into a
    /// handful of unreadable XML fragments.
    #[test]
    fn an_ofd_is_not_unpacked_as_if_it_were_a_batch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("发票.ofd");
        {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            writer
                .start_file::<_, ()>("OFD.xml", Default::default())
                .unwrap();
            writer.write_all(b"<ofd:OFD/>").unwrap();
            writer.finish().unwrap();
        }

        let files = expand(&[path]);
        assert_eq!(files.len(), 1, "OFD 应作为单个文件导入");
        assert!(files[0].0.ends_with(".ofd"));
    }

    #[test]
    fn unreadable_paths_are_skipped_rather_than_aborting_the_import() {
        let files = expand(&[PathBuf::from("/nonexistent/nowhere.pdf")]);
        assert!(files.is_empty());
    }
}
