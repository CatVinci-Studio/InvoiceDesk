/**
 * The shapes the Rust side sends and expects.
 *
 * Mirrored by hand from `src-tauri/src/model.rs`, `db/mod.rs`, `classify/`
 * and `commands/` - comments on both sides point here. Serde is configured
 * `rename_all = "camelCase"`, so field names match one-for-one.
 */

// ---------------------------------------------------------------------------
// Money
// ---------------------------------------------------------------------------

/**
 * An amount in 分 (cents), never yuan.
 *
 * The Rust side stores money as an integer count of cents precisely so that
 * no rounding can happen to it, and that guarantee only holds if this side
 * keeps it integral too. Divide by 100 to DISPLAY (see `formatMoney`); never
 * to compute. Anything that adds amounts adds cents.
 */
export type Cents = number;

// ---------------------------------------------------------------------------
// Field provenance
// ---------------------------------------------------------------------------

/** Mirrors `FieldSource`, ordered worst-to-best. */
export type FieldSource =
  "missing" | "vision" | "pdfText" | "qrCode" | "xml" | "manual";

/** Below this a field is flagged for review. Mirrors `REVIEW_THRESHOLD`. */
export const REVIEW_THRESHOLD = 0.9;

export const SOURCE_LABEL: Record<FieldSource, string> = {
  missing: "未识别",
  vision: "AI 识别",
  pdfText: "PDF 文本层",
  qrCode: "二维码",
  xml: "发票 XML",
  manual: "人工录入",
};

/** One extracted value plus where it came from. */
export interface Field<T> {
  value: T | null;
  confidence: number;
  source: FieldSource;
}

export function emptyField<T>(): Field<T> {
  return { value: null, confidence: 0, source: "missing" };
}

/**
 * A value the user typed. Manual beats every machine source in
 * `Field::merge_from`, which is what stops a re-scan from undoing a
 * correction - so edits MUST go through this rather than only setting
 * `value`.
 */
export function manualField<T>(value: T): Field<T> {
  return { value, confidence: 1, source: "manual" };
}

export function needsReview<T>(field: Field<T>): boolean {
  return field.value === null || field.confidence < REVIEW_THRESHOLD;
}

// ---------------------------------------------------------------------------
// Invoice
// ---------------------------------------------------------------------------

export type InvoiceKind =
  | "digitalSpecial"
  | "digitalGeneral"
  | "vatSpecial"
  | "vatGeneral"
  | "vatElectronicGeneral"
  | "vatRoll"
  | "train"
  | "airItinerary"
  | "taxi"
  | "fixedAmount"
  | "toll"
  | "other";

export const KIND_LABEL: Record<InvoiceKind, string> = {
  digitalSpecial: "数电票（专用）",
  digitalGeneral: "数电票（普通）",
  vatSpecial: "增值税专用发票",
  vatGeneral: "增值税普通发票",
  vatElectronicGeneral: "增值税电子普通发票",
  vatRoll: "增值税普通发票（卷式）",
  train: "铁路电子客票",
  airItinerary: "航空运输电子客票行程单",
  taxi: "出租车票",
  fixedAmount: "定额发票",
  toll: "过路过桥费发票",
  other: "其他票据",
};

export interface InvoiceItem {
  name: string;
  spec: string | null;
  unit: string | null;
  quantity: string | null;
  unitPrice: Cents | null;
  amount: Cents | null;
  taxRate: string | null;
  tax: Cents | null;
}

/** Mirrors `ValidationIssue`, a serde-tagged enum. */
export type ValidationIssue =
  | { kind: "totalMismatch"; detail: { expected: Cents; found: Cents } }
  | { kind: "numberLength"; detail: { found: number; expected: number } }
  | { kind: "badDate"; detail: { raw: string } }
  | { kind: "futureDate"; detail: { date: string } }
  | { kind: "missingField"; detail: { field: string } };

export interface Invoice {
  id: number | null;
  kind: InvoiceKind | null;
  number: Field<string>;
  code: Field<string>;
  issuedOn: Field<string>;
  checkCode: Field<string>;
  buyerName: Field<string>;
  buyerTaxId: Field<string>;
  sellerName: Field<string>;
  sellerTaxId: Field<string>;
  amountExclTax: Field<Cents>;
  tax: Field<Cents>;
  total: Field<Cents>;
  items: InvoiceItem[];
  remark: Field<string>;
  category: string | null;
  categoryRule: string | null;
  sourcePath: string;
  fileHash: string;
  issues: ValidationIssue[];
}

/** A row of the list - the projection `db::list` returns. */
export interface InvoiceRow {
  id: number;
  number: string;
  issuedOn: string;
  kind: string;
  sellerName: string;
  totalCents: Cents;
  taxCents: Cents;
  category: string;
  categoryRule: string;
  minConfidence: number;
  issueCount: number;
  reviewed: boolean;
  sourcePath: string;
  /** Other rows carrying the same 发票代码+号码. Non-empty = 疑似重复报销. */
  duplicateOf: number[];
}

export interface InvoiceFilter {
  search?: string | null;
  category?: string | null;
  from?: string | null;
  to?: string | null;
  needsReviewOnly?: boolean;
  duplicatesOnly?: boolean;
  excludeReport?: number | null;
}

export interface CategoryTotal {
  category: string;
  count: number;
  totalCents: Cents;
  taxCents: Cents;
}

// ---------------------------------------------------------------------------
// Classification rules
// ---------------------------------------------------------------------------

export type MatchField =
  "taxCategory" | "itemName" | "sellerName" | "remark" | "kind";

export const MATCH_FIELD_LABEL: Record<MatchField, string> = {
  taxCategory: "税收分类简称",
  itemName: "项目名称",
  sellerName: "销售方名称",
  remark: "备注",
  kind: "票种",
};

/**
 * `taxCategory` is the `*...*` prefix every VAT line item starts with - the
 * tax authority's own classification, and therefore the most reliable thing
 * to match on. The rule editor should say so where the field is chosen.
 */
export const MATCH_FIELD_HINT: Record<MatchField, string> = {
  taxCategory: "发票项目里 *星号* 之间的部分，税局统一分类，最可靠",
  itemName: "完整的货物或服务名称",
  sellerName: "开票公司名称，最容易误伤，建议放低优先级",
  remark: "发票备注栏",
  kind: "发票种类，例如铁路电子客票",
};

export interface Condition {
  field: MatchField;
  keywords: string[];
}

export interface Rule {
  id: number | null;
  name: string;
  category: string;
  /** Higher wins. 150 = 分类简称, 100 = 票种, 50 = 销售方兜底, 200 = AI 建议。 */
  priority: number;
  enabled: boolean;
  conditions: Condition[];
}

export interface Suggestion {
  category: string;
  reason: string;
  proposedRule: Rule;
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

export interface Report {
  id: number | null;
  title: string;
  applicant: string;
  department: string;
  note: string;
  createdAt: string;
  invoiceCount: number;
  totalCents: Cents;
}

export interface ReportMeta {
  title: string;
  applicant: string;
  department: string;
  note: string;
  date: string;
}

export interface ReportPreview {
  count: number;
  amountExclTaxCents: Cents;
  taxCents: Cents;
  totalCents: Cents;
  deductibleTaxCents: Cents;
  capitalAmount: string;
  byCategory: { category: string; count: number; totalCents: Cents }[];
  /** `[发票号码, 问题]` pairs. */
  warnings: [string, string][];
  /** 发票号码 of rows that also exist elsewhere in the ledger. */
  duplicates: string[];
}

export interface FillReport {
  filled: string[];
  missing: string[];
  detailRows: number;
}

// ---------------------------------------------------------------------------
// AI
// ---------------------------------------------------------------------------

export interface ProviderCatalogEntry {
  id: string;
  label: string;
  consoleUrl: string;
  baseUrl: string | null;
  visionModel: string | null;
  textModel: string | null;
  keyOptional: boolean;
  modelsListable: boolean;
  knownModels: string[];
}

export interface AiSettings {
  /** Off by default: turning it on means invoice IMAGES leave the machine. */
  visionEnabled: boolean;
  /** Off by default: turning it on means the seller name leaves the machine. */
  suggestEnabled: boolean;
  provider: string;
  visionModel: string | null;
  textModel: string | null;
  customBaseUrl: string | null;
}

export const DEFAULT_AI_SETTINGS: AiSettings = {
  visionEnabled: false,
  suggestEnabled: false,
  provider: "qwen",
  visionModel: null,
  textModel: null,
  customBaseUrl: null,
};

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

export type FileStatus =
  "imported" | "needsReview" | "alreadyImported" | "failed";

export const FILE_STATUS_LABEL: Record<FileStatus, string> = {
  imported: "已导入",
  needsReview: "待复核",
  alreadyImported: "重复文件",
  failed: "识别失败",
};

/** One step of the extraction pipeline, for the 识别过程 disclosure. */
export interface TraceStep {
  layer: string;
  hit: boolean;
  detail: string;
}

/** Mirrors `IngestEvent` in `src-tauri/src/commands/ingest.rs`. */
export type IngestEvent =
  | { type: "started"; total: number }
  | {
      type: "file";
      index: number;
      name: string;
      status: FileStatus;
      invoiceId: number | null;
      trace: TraceStep[];
      message: string;
      /** The vision fallback would have helped here, but is switched off. */
      visionWouldHelp: boolean;
    }
  | {
      type: "finished";
      imported: number;
      needsReview: number;
      duplicates: number;
      failed: number;
    };
