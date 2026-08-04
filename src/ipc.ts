/**
 * Typed wrappers over every Tauri command.
 *
 * One place where a command name is spelled, so a rename in
 * `src-tauri/src/lib.rs` breaks compilation here rather than failing at
 * runtime in whichever pane happened to call it. Grouped to match the
 * command modules on the Rust side.
 */

import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AiSettings,
  CategoryTotal,
  FillReport,
  IngestEvent,
  Invoice,
  InvoiceFilter,
  InvoiceRow,
  ProviderCatalogEntry,
  Report,
  ReportMeta,
  ReportPreview,
  Rule,
  Suggestion,
} from "./types";

export const ingest = {
  /**
   * Imports files or folders. `onEvent` fires per file as it lands - see
   * `IngestEvent`. Resolves once every file has been processed.
   */
  async importFiles(
    paths: string[],
    onEvent: (event: IngestEvent) => void,
  ): Promise<void> {
    const channel = new Channel<IngestEvent>();
    channel.onmessage = onEvent;
    return invoke("import_files", { paths, channel });
  },
};

export const invoices = {
  list: (filter: InvoiceFilter): Promise<InvoiceRow[]> =>
    invoke("list_invoices", { filter }),

  totals: (filter: InvoiceFilter): Promise<CategoryTotal[]> =>
    invoke("invoice_totals", { filter }),

  get: (id: number): Promise<Invoice | null> => invoke("get_invoice", { id }),

  /**
   * Saves corrections. Fields the user edited must carry
   * `source: "manual"` (see `manualField`) or a later re-scan will overwrite
   * them.
   */
  save: (id: number, invoice: Invoice, reviewed: boolean): Promise<void> =>
    invoke("save_invoice", { id, invoice, reviewed }),

  remove: (id: number): Promise<void> => invoke("delete_invoice", { id }),

  setCategory: (id: number, category: string): Promise<void> =>
    invoke("set_invoice_category", { id, category }),

  /** Re-runs the rules over stored invoices. Returns how many changed. */
  reclassifyAll: (): Promise<number> => invoke("reclassify_all"),

  /** Asks the model for a category. Never applies it - see `Suggestion`. */
  suggestCategory: (id: number): Promise<Suggestion> =>
    invoke("suggest_category", { id }),

  /** Re-reads the original file. Manual corrections survive. */
  rescan: (id: number): Promise<Invoice> => invoke("rescan_invoice", { id }),

  openSource: (path: string): Promise<void> =>
    invoke("open_source_file", { path }),
};

export const rules = {
  list: (): Promise<Rule[]> => invoke("list_rules"),
  save: (rule: Rule): Promise<number> => invoke("save_rule", { rule }),
  remove: (id: number): Promise<void> => invoke("delete_rule", { id }),
  categories: (): Promise<string[]> => invoke("list_categories"),
  /** Puts back built-in rules the user does not currently have. */
  restoreDefaults: (): Promise<number> => invoke("restore_default_rules"),

  /**
   * The rule set as a JSON string / from a JSON string.
   *
   * The file-based pair below is what the rule pane actually calls; these two
   * stay because they are the primitive underneath it - the JSON never has to
   * touch the disk to be tested, and they are the shape anything that wants
   * the rules in memory should use.
   */
  exportJson: (): Promise<string> => invoke("export_rules"),
  importJson: (json: string): Promise<number> =>
    invoke("import_rules", { json }),

  /**
   * Same, but the backend does the file IO - the split the report export
   * already uses. The frontend picks the path with `plugin-dialog` and hands
   * it over, so the app needs no filesystem permission of its own. Both
   * return how many rules were written or added.
   */
  exportToFile: (path: string): Promise<number> =>
    invoke("export_rules_to_file", { path }),
  importFromFile: (path: string): Promise<number> =>
    invoke("import_rules_from_file", { path }),
};

export const reports = {
  list: (): Promise<Report[]> => invoke("list_reports"),
  save: (report: Report): Promise<number> => invoke("save_report", { report }),
  remove: (id: number): Promise<void> => invoke("delete_report", { id }),

  setInvoices: (reportId: number, invoiceIds: number[]): Promise<void> =>
    invoke("set_report_invoices", { reportId, invoiceIds }),

  invoices: (reportId: number): Promise<Invoice[]> =>
    invoke("report_invoices", { reportId }),

  /** Totals plus every problem that would ride into the sheet. */
  preview: (reportId: number): Promise<ReportPreview> =>
    invoke("preview_report", { reportId }),

  exportXlsx: (
    reportId: number,
    meta: ReportMeta,
    outPath: string,
  ): Promise<void> => invoke("export_report", { reportId, meta, outPath }),

  exportWithTemplate: (
    reportId: number,
    meta: ReportMeta,
    templatePath: string,
    outPath: string,
  ): Promise<FillReport> =>
    invoke("export_report_with_template", {
      reportId,
      meta,
      templatePath,
      outPath,
    }),

  templatePlaceholders: (): Promise<[string, string][]> =>
    invoke("template_placeholders"),
};

export const ai = {
  providers: (): Promise<ProviderCatalogEntry[]> => invoke("list_providers"),
  getSettings: (): Promise<AiSettings> => invoke("get_ai_settings"),
  setSettings: (settings: AiSettings): Promise<void> =>
    invoke("set_ai_settings", { settings }),
  /** Returns a human-readable success line, or rejects with why not. */
  testConnection: (): Promise<string> => invoke("test_ai_connection"),
  listModels: (): Promise<string[]> => invoke("list_provider_models"),

  setKey: (provider: string, key: string): Promise<void> =>
    invoke("set_provider_api_key", { provider, key }),
  hasKey: (provider: string): Promise<boolean> =>
    invoke("provider_api_key_status", { provider }),
  clearKey: (provider: string): Promise<void> =>
    invoke("clear_provider_api_key", { provider }),
};

export const prefs = {
  get: (key: string): Promise<string | null> =>
    invoke("get_preference", { key }),
  set: (key: string, value: string): Promise<void> =>
    invoke("set_preference", { key, value }),
};

/**
 * Normalises whatever a rejected `invoke` threw into a string.
 *
 * Tauri rejects with the command's `Err` payload, which for every command
 * here is already a Chinese sentence written for the user - so the common
 * case is a plain string and this just passes it through. The fallbacks
 * exist so a panic or a serialisation failure surfaces as *something*
 * rather than "[object Object]".
 */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error ?? "未知错误");
}
