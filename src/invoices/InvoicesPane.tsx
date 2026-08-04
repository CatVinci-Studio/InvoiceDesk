/**
 * 全部发票 / 待复核 / 疑似重复 - one screen, three entry points.
 *
 * The scope the sidebar picked is a *forced* filter, not a default: in the
 * 待复核 view the 只看待复核 control is not merely on, it is absent, because
 * the sidebar already said what this view is and a toggle that can silently
 * contradict the heading above it is a lie waiting to happen.
 *
 * The one invariant everything else hangs off: **the table and the status bar
 * are computed from the same `InvoiceFilter` object.** `invoices.list` and
 * `invoices.totals` take it unchanged (and `totals_by_category` is itself
 * implemented on top of `list`, for the same reason), so the running total
 * under the table always describes exactly the rows above it. Sorting happens
 * afterwards and client-side, where it cannot affect either.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { formatMoney, monthsAgo, today } from "../format";
import {
  errorMessage,
  invoices as invoicesApi,
  rules as rulesApi,
} from "../ipc";
import type { CategoryTotal, InvoiceFilter, InvoiceRow } from "../types";
import {
  Button,
  Modal,
  SearchInput,
  Select,
  Toggle,
  useAsync,
  useToast,
} from "../ui/primitives";
import { DateInput } from "./DateInput";
import { InvoiceDetail } from "./InvoiceDetail";
import { InvoiceTable } from "./InvoiceTable";
import {
  DEFAULT_SORT,
  UNCATEGORISED,
  categoryLabel,
  sortRows,
  type Sort,
} from "./rows";
import "./invoices.css";

export interface InvoicesPaneProps {
  scope: "all" | "review" | "duplicates";
  /** Called after anything that moves a count, so the sidebar badges follow. */
  onChanged: () => void;
  /** For panes that want to act on the selection (报销单, later). */
  onSelectionChange?: (ids: number[]) => void;
}

/**
 * The value space of the two category selects in this pane.
 *
 * A real category is offered as `=名称`; the two bare words below are the
 * entries that are not categories at all. The `=` prefix is what makes that
 * safe - a category name is user-typed text, so any bare sentinel could one
 * day be somebody's actual category.
 *
 * The empty string is deliberately NOT a sentinel: it is the real stored
 * value of an unclassified row (`db::update` writes `""` for
 * `category: None`), so `category: ""` in a filter genuinely means 未分类,
 * while `null` means "no category constraint at all".
 */
const ANY_CATEGORY = "all";
const PICK_CATEGORY = "pick";

const asOption = (category: string) => `=${category}`;
const fromOption = (value: string) => value.slice(1);

/** 拼音 order, so a list of categories reads the way a Chinese reader expects. */
const COLLATOR = new Intl.Collator("zh-CN");

/** Rows that share a 发票号码 with another, regardless of the current filter. */
const DUPLICATE_FILTER: InvoiceFilter = {
  needsReviewOnly: false,
  duplicatesOnly: true,
};

type RangePreset =
  "all" | "thisMonth" | "lastMonth" | "last3" | "thisYear" | "custom";

const RANGE_OPTIONS: { value: RangePreset; label: string }[] = [
  { value: "all", label: "全部时间" },
  { value: "thisMonth", label: "本月" },
  { value: "lastMonth", label: "上月" },
  { value: "last3", label: "近三月" },
  { value: "thisYear", label: "今年" },
  { value: "custom", label: "自定义" },
];

/** `YYYY-MM-DD` from local date parts - never `toISOString`, which is UTC and
 *  shifts the date by a day for anyone east of Greenwich, which is everyone
 *  this app is for. */
function localDate(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function rangeOf(preset: RangePreset): { from: string; to: string } | null {
  const now = new Date();
  switch (preset) {
    case "all":
      return { from: "", to: "" };
    case "thisMonth":
      return { from: monthsAgo(0), to: today() };
    case "lastMonth":
      // Day 0 of this month is the last day of the previous one.
      return {
        from: monthsAgo(1),
        to: localDate(new Date(now.getFullYear(), now.getMonth(), 0)),
      };
    case "last3":
      return { from: monthsAgo(2), to: today() };
    case "thisYear":
      return { from: `${now.getFullYear()}-01-01`, to: today() };
    case "custom":
      return null;
  }
}

export function InvoicesPane({
  scope,
  onChanged,
  onSelectionChange,
}: InvoicesPaneProps) {
  const toast = useToast();

  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [preset, setPreset] = useState<RangePreset>("all");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [category, setCategory] = useState<string>(ANY_CATEGORY);
  const [reviewOnly, setReviewOnly] = useState(false);
  const [sort, setSort] = useState<Sort>(DEFAULT_SORT);
  const [selected, setSelected] = useState<Set<number>>(() => new Set());
  const [openId, setOpenId] = useState<number | null>(null);
  const [detailDirty, setDetailDirty] = useState(false);
  const [confirmBulkDelete, setConfirmBulkDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  /** Bumped by anything that changes stored data, to re-run every query. */
  const [nonce, setNonce] = useState(0);

  // Typing is faster than SQLite answers. `useAsync` already discards stale
  // responses, but debouncing means we do not fire eight queries to show the
  // result of the eighth.
  useEffect(() => {
    const timer = setTimeout(() => setSearch(searchInput), 180);
    return () => clearTimeout(timer);
  }, [searchInput]);

  // Switching views is a change of subject: a selection made in 全部发票 has
  // no meaning in 疑似重复, and leaving the drawer open on an invoice that is
  // no longer in the list is disorienting.
  useEffect(() => {
    setSelected(new Set());
    setOpenId(null);
  }, [scope]);

  const filter = useMemo<InvoiceFilter>(
    () => ({
      search: search.trim() === "" ? null : search.trim(),
      category: category === ANY_CATEGORY ? null : fromOption(category),
      from: from === "" ? null : from,
      to: to === "" ? null : to,
      // Both flags are always sent, never omitted: `db::Filter` declares them
      // as plain `bool`, and serde only fills in missing fields for `Option`.
      needsReviewOnly: scope === "review" || (scope === "all" && reviewOnly),
      duplicatesOnly: scope === "duplicates",
    }),
    [search, category, from, to, scope, reviewOnly],
  );

  const rowsQuery = useAsync<InvoiceRow[]>(
    () => invoicesApi.list(filter),
    [filter, nonce],
    [],
  );
  const totalsQuery = useAsync<CategoryTotal[]>(
    () => invoicesApi.totals(filter),
    [filter, nonce],
    [],
  );
  const duplicatesQuery = useAsync<InvoiceRow[]>(
    () => invoicesApi.list(DUPLICATE_FILTER),
    [nonce],
    [],
  );
  const categoriesQuery = useAsync<string[]>(
    () => rulesApi.categories(),
    [nonce],
    [],
  );

  const refresh = useCallback(() => {
    setNonce((current) => current + 1);
    onChanged();
  }, [onChanged]);

  const rows = useMemo(
    () => sortRows(rowsQuery.data, sort),
    [rowsQuery.data, sort],
  );

  /** Every duplicate row in the ledger, so the popover can describe a twin
   *  the current filter is hiding. */
  const duplicateIndex = useMemo(() => {
    const index = new Map<number, InvoiceRow>();
    for (const row of duplicatesQuery.data) index.set(row.id, row);
    return index;
  }, [duplicatesQuery.data]);

  /**
   * Category names for every picker in the pane.
   *
   * Drawn from the rules (so a category exists before any invoice has it) and
   * from the rows on screen (so a category whose rule was deleted is still
   * selectable). 未分类 is dropped here and re-added by each picker as the
   * empty-string option - see the note on `ANY_CATEGORY`.
   */
  const categories = useMemo(() => {
    const names = new Set<string>();
    for (const name of categoriesQuery.data) {
      if (name && name !== UNCATEGORISED) names.add(name);
    }
    for (const row of rowsQuery.data) {
      if (row.category && row.category !== UNCATEGORISED)
        names.add(row.category);
    }
    return [...names].sort(COLLATOR.compare);
  }, [categoriesQuery.data, rowsQuery.data]);

  // A selection has to mean "these rows, the ones I can see". Rows that the
  // filter no longer returns drop out of it rather than riding along
  // invisibly into a bulk delete.
  useEffect(() => {
    setSelected((current) => {
      if (current.size === 0) return current;
      const visible = new Set(rowsQuery.data.map((row) => row.id));
      const next = new Set([...current].filter((id) => visible.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [rowsQuery.data]);

  useEffect(() => {
    onSelectionChange?.([...selected]);
  }, [selected, onSelectionChange]);

  const openIndex =
    openId === null ? -1 : rows.findIndex((row) => row.id === openId);
  const openRow = openIndex >= 0 ? rows[openIndex] : undefined;

  const open = useCallback(
    (id: number) => {
      // The drawer guards its own exits; this is the one path it cannot see.
      if (detailDirty && openId !== null && id !== openId) {
        toast("这张发票还有未保存的修改，请先保存或放弃。", "error");
        return;
      }
      setOpenId(id);
    },
    [detailDirty, openId, toast],
  );

  const navigate = useCallback(
    (delta: number) => {
      if (openIndex < 0) return;
      const next = rows[openIndex + delta];
      if (next) setOpenId(next.id);
    },
    [openIndex, rows],
  );

  const setRowCategory = useCallback(
    async (id: number, value: string) => {
      try {
        await invoicesApi.setCategory(id, value);
        refresh();
      } catch (error) {
        toast(errorMessage(error), "error");
      }
    },
    [refresh, toast],
  );

  const applyBulkCategory = useCallback(
    async (value: string) => {
      const ids = [...selected];
      setBusy(true);
      try {
        // Sequential on purpose: every one of these takes the same SQLite
        // connection lock, so firing them in parallel only queues them behind
        // each other while making a partial failure harder to report.
        for (const id of ids) await invoicesApi.setCategory(id, value);
        toast(`已把 ${ids.length} 张设为「${categoryLabel(value)}」`);
        refresh();
      } catch (error) {
        toast(errorMessage(error), "error");
      } finally {
        setBusy(false);
      }
    },
    [selected, refresh, toast],
  );

  const applyBulkDelete = useCallback(async () => {
    const ids = [...selected];
    setBusy(true);
    try {
      for (const id of ids) await invoicesApi.remove(id);
      if (openId !== null && ids.includes(openId)) setOpenId(null);
      setSelected(new Set());
      setConfirmBulkDelete(false);
      toast(`已删除 ${ids.length} 张`);
      refresh();
    } catch (error) {
      toast(errorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  }, [selected, openId, refresh, toast]);

  const summary = useMemo(
    () =>
      totalsQuery.data.reduce(
        (accumulator, entry) => ({
          count: accumulator.count + entry.count,
          totalCents: accumulator.totalCents + entry.totalCents,
          taxCents: accumulator.taxCents + entry.taxCents,
          uncategorised:
            accumulator.uncategorised +
            (entry.category === UNCATEGORISED ? entry.count : 0),
        }),
        { count: 0, totalCents: 0, taxCents: 0, uncategorised: 0 },
      ),
    [totalsQuery.data],
  );

  const selectedTotal = useMemo(
    () =>
      rows.reduce(
        (sum, row) => (selected.has(row.id) ? sum + row.totalCents : sum),
        0,
      ),
    [rows, selected],
  );

  const categoryFilterOptions = useMemo(
    () => [
      { value: ANY_CATEGORY, label: "全部类别" },
      ...categories.map((name) => ({ value: asOption(name), label: name })),
      { value: asOption(""), label: UNCATEGORISED },
    ],
    [categories],
  );

  const bulkCategoryOptions = useMemo(
    () => [
      { value: PICK_CATEGORY, label: "设为类别…" },
      ...categories.map((name) => ({ value: asOption(name), label: name })),
      { value: asOption(""), label: UNCATEGORISED },
    ],
    [categories],
  );

  const filtersActive =
    search.trim() !== "" ||
    category !== ANY_CATEGORY ||
    from !== "" ||
    to !== "" ||
    reviewOnly;

  return (
    <div className="invoices-pane">
      <div className="toolbar">
        <SearchInput value={searchInput} onChange={setSearchInput} />

        <div className="range-control">
          <Select
            value={preset}
            options={RANGE_OPTIONS}
            onChange={(value) => {
              setPreset(value);
              const range = rangeOf(value);
              if (range) {
                setFrom(range.from);
                setTo(range.to);
              }
            }}
          />
          <DateInput
            value={from}
            title="起始开票日期"
            onChange={(value) => {
              setFrom(value);
              setPreset("custom");
            }}
          />
          <span className="range-dash">至</span>
          <DateInput
            value={to}
            title="截止开票日期"
            onChange={(value) => {
              setTo(value);
              setPreset("custom");
            }}
          />
        </div>

        <Select
          value={category}
          options={categoryFilterOptions}
          onChange={setCategory}
        />

        <span className="toolbar-spacer" />

        {/* Only in 全部发票: the other two scopes ARE this filter, and a
            control that could contradict the view's own name is worse than
            no control. */}
        {scope === "all" && (
          <span className="toolbar-toggle">
            <span>只看待复核</span>
            <Toggle checked={reviewOnly} onChange={setReviewOnly} />
          </span>
        )}
      </div>

      {selected.size > 0 && (
        <div className="bulkbar">
          <span className="bulkbar-count">已选 {selected.size} 张</span>
          <Select
            value={PICK_CATEGORY}
            disabled={busy}
            options={bulkCategoryOptions}
            onChange={(value) => {
              if (value === PICK_CATEGORY) return;
              void applyBulkCategory(fromOption(value));
            }}
          />
          <Button
            intent="danger"
            disabled={busy}
            onClick={() => setConfirmBulkDelete(true)}
          >
            删除
          </Button>
          <span className="toolbar-spacer" />
          <Button disabled={busy} onClick={() => setSelected(new Set())}>
            取消选择
          </Button>
        </div>
      )}

      <div className="invoices-body">
        {rowsQuery.error ? (
          <div className="empty">
            <div className="empty-title">读取发票失败</div>
            <div className="empty-hint">{rowsQuery.error}</div>
          </div>
        ) : rows.length === 0 ? (
          <div className="empty">
            {rowsQuery.loading ? (
              <div className="empty-hint">读取中…</div>
            ) : (
              <>
                <div className="empty-title">
                  {filtersActive
                    ? "没有符合条件的发票"
                    : scope === "review"
                      ? "没有需要复核的发票"
                      : scope === "duplicates"
                        ? "没有疑似重复的发票"
                        : "还没有发票"}
                </div>
                <div className="empty-hint">
                  {filtersActive
                    ? "试着把日期范围放宽，或者把类别改回「全部类别」。"
                    : scope === "review"
                      ? "识别置信度都够高，也没有校验问题。新导入的发票如果有疑点会出现在这里。"
                      : scope === "duplicates"
                        ? "没有两张发票共用同一个发票代码和号码。"
                        : "把 PDF、OFD、图片或压缩包拖进窗口，或者点左上角的「导入发票」。"}
                </div>
              </>
            )}
          </div>
        ) : (
          <InvoiceTable
            rows={rows}
            sort={sort}
            onSortChange={setSort}
            selected={selected}
            onSelectedChange={setSelected}
            activeId={openId}
            onOpen={open}
            categories={categories}
            onCategoryChange={(id, value) => void setRowCategory(id, value)}
            duplicateIndex={duplicateIndex}
          />
        )}

        {openId !== null && (
          <InvoiceDetail
            // Keyed by id so each invoice gets a clean form: no draft, no
            // half-typed amount and no stale AI suggestion carried across.
            key={openId}
            id={openId}
            // A row reached through the duplicate popover can be one the
            // current filter hides, and then its stored review state is
            // simply not on hand; false is the safe assumption, since the
            // worst it costs is one more pass through 待复核.
            reviewed={openRow?.reviewed ?? false}
            categories={categories}
            index={openIndex}
            count={rows.length}
            hasPrev={openIndex > 0}
            hasNext={openIndex >= 0 && openIndex < rows.length - 1}
            onNavigate={navigate}
            onClose={() => setOpenId(null)}
            onChanged={refresh}
            onDirtyChange={setDetailDirty}
          />
        )}
      </div>

      <div className="statusbar">
        <span>
          张数 <strong>{summary.count}</strong>
        </span>
        <span>
          合计金额 <strong>{formatMoney(summary.totalCents)}</strong>
        </span>
        <span>
          税额合计 <strong>{formatMoney(summary.taxCents)}</strong>
        </span>
        {summary.uncategorised > 0 && (
          <span>未分类 {summary.uncategorised} 张</span>
        )}
        <span className="statusbar-spacer" />
        {selected.size > 0 && (
          <span>
            已选 <strong>{selected.size}</strong> 张 ·{" "}
            <strong>{formatMoney(selectedTotal)}</strong>
          </span>
        )}
      </div>

      {confirmBulkDelete && (
        <Modal
          title={`删除 ${selected.size} 张发票`}
          onClose={() => setConfirmBulkDelete(false)}
          footer={
            <>
              <Button onClick={() => setConfirmBulkDelete(false)}>取消</Button>
              <Button
                intent="danger"
                disabled={busy}
                onClick={() => void applyBulkDelete()}
              >
                删除
              </Button>
            </>
          }
        >
          只从智税的账本里删除这些记录，原始文件不会被动。
        </Modal>
      )}
    </div>
  );
}
