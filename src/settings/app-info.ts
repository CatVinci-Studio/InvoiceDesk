/**
 * Who this app is, for the 「关于」 section.
 *
 * The version is a hand-kept copy of the one in `src-tauri/tauri.conf.json`
 * (and `package.json`), which is the number the installer, the updater
 * manifest and the crash reports all speak. It is duplicated here rather than
 * imported because `tauri.conf.json` sits outside `tsconfig.json`'s `include`
 * and importing across that boundary drags the whole `src-tauri` tree into
 * the type graph for one string. `app-info.test.ts` reads the real files and
 * fails if the copy drifts, which is the part that actually matters - an
 * about box quoting a version the user does not have is worse than none.
 */

/**
 * The name shown inside the app.
 *
 * Chinese, because every other word in this interface is. The Latin
 * `InvoiceDesk` is kept for things a filesystem and a URL have to carry - the
 * package name, the bundle identifier, `InvoiceDesk_0.0.1_aarch64.dmg`, the
 * updater manifest - where a CJK name would be percent-encoded in every link
 * and awkward in every shell.
 */
export const APP_NAME = "智票";
/** The romanisation, for filenames and the repository. */
export const APP_NAME_LATIN = "Invoice Desk";
export const APP_VERSION = "0.0.4";
export const APP_VENDOR = "CatVinci Studio";

/** The ledger. Named here so the 「关于」 section can point a user at the one
 *  file worth backing up. Mirrors `db_path` in `src-tauri/src/lib.rs`. */
export const LEDGER_FILE_NAME = "invoicedesk.db";
