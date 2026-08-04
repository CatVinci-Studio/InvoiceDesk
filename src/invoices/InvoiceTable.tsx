/**
 * The ledger itself.
 *
 * Design notes that are decisions rather than code:
 *
 * **No virtualisation.** A personal expense ledger is hundreds of rows across
 * a year, not millions; the browser lays a thousand `<tr>`s out in a frame or
 * two. A virtual list would buy nothing measurable and would cost the two
 * things this table actually needs - `Ctrl+F` finding a 发票号码 that is
 * scrolled out of view, and `scrollIntoView` on the row the drawer is showing.
 * If the row count ever reaches five figures, revisit; until then this is the
 * cheaper design, not the lazier one.
 *
 * **Colour only where a person is needed.** A clean row shows an EMPTY status
 * cell - no green tick, no 已确认 badge. Most rows are fine, and a column of
 * green ticks is a wall the eye stops reading after ten rows, which is
 * precisely when the one amber badge in the middle of it stops being visible.
 * The status column is budgeted for exactly two states: 疑似重复 (danger, this
 * may already have been claimed) and 待复核 (warn, a number here is a guess).
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
} from "react";
import { formatAmount, formatDate, formatMoney } from "../format";
import type { InvoiceRow } from "../types";
import { ChevronIcon } from "../ui/icons";
import { Badge, Select } from "../ui/primitives";
import { useCloseOnOutsideClick } from "../utils/useCloseOnOutsideClick";
import {
  amountExclTaxOf,
  rowIsDuplicate,
  rowNeedsReview,
  UNCATEGORISED,
  type Sort,
  type SortKey,
} from "./rows";

export interface InvoiceTableProps {
  rows: InvoiceRow[];
  sort: Sort;
  onSortChange: (sort: Sort) => void;
  selected: ReadonlySet<number>;
  onSelectedChange: (next: Set<number>) => void;
  /** The row the detail drawer is showing, scrolled into view as it moves. */
  activeId: number | null;
  onOpen: (id: number) => void;
  /** Category names offered by the inline cell editor, 未分类 excluded. */
  categories: string[];
  onCategoryChange: (id: number, category: string) => void;
  /**
   * Every row that shares a 发票代码+号码 with another, by id - including the
   * ones the current filter hides. The duplicate popover has to be able to
   * describe a twin the user cannot currently see, which is the whole point
   * of it.
   */
  duplicateIndex: ReadonlyMap<number, InvoiceRow>;
}

/** Where the 疑似重复 popover is pinned, in viewport coordinates. */
interface DuplicatePopover {
  row: InvoiceRow;
  left: number;
  top: number;
}

export function InvoiceTable({
  rows,
  sort,
  onSortChange,
  selected,
  onSelectedChange,
  activeId,
  onOpen,
  categories,
  onCategoryChange,
  duplicateIndex,
}: InvoiceTableProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const headerCheckRef = useRef<HTMLInputElement>(null);
  /** Where the last selection click landed, for shift-click ranges. */
  const anchor = useRef<number | null>(null);

  const [popover, setPopover] = useState<DuplicatePopover | null>(null);
  const closePopover = useCallback(() => setPopover(null), []);
  const popoverRef = useCloseOnOutsideClick<HTMLDivElement>(closePopover, true);

  // Follow the drawer. `block: "nearest"` rather than "center" so arrowing
  // down a screenful of rows scrolls one row at a time instead of yanking the
  // list to the middle on every keypress.
  useEffect(() => {
    if (activeId === null) return;
    const element = scrollRef.current?.querySelector(
      `[data-row-id="${activeId}"]`,
    );
    element?.scrollIntoView({ block: "nearest" });
  }, [activeId, rows]);

  const allSelected =
    rows.length > 0 && rows.every((row) => selected.has(row.id));
  const anySelected = rows.some((row) => selected.has(row.id));

  // `indeterminate` is a DOM property with no HTML attribute, so it can only
  // be set imperatively - the half-filled box is what tells the user the
  // header checkbox will ADD to their selection rather than replace it.
  useEffect(() => {
    if (headerCheckRef.current) {
      headerCheckRef.current.indeterminate = anySelected && !allSelected;
    }
  }, [anySelected, allSelected]);

  const selectRange = useCallback(
    (index: number) => {
      const start = anchor.current === null ? index : anchor.current;
      const [low, high] = start <= index ? [start, index] : [index, start];
      const next = new Set(selected);
      for (let i = low; i <= high; i += 1) next.add(rows[i].id);
      onSelectedChange(next);
    },
    [rows, selected, onSelectedChange],
  );

  const toggleOne = useCallback(
    (id: number) => {
      const next = new Set(selected);
      if (!next.delete(id)) next.add(id);
      onSelectedChange(next);
    },
    [selected, onSelectedChange],
  );

  const onHeaderCheck = useCallback(() => {
    if (allSelected) {
      // Clears only what is on screen; a selection made under a different
      // filter is not visible here and silently dropping it would be a
      // surprise.
      const next = new Set(selected);
      for (const row of rows) next.delete(row.id);
      onSelectedChange(next);
    } else {
      onSelectedChange(new Set([...selected, ...rows.map((row) => row.id)]));
    }
  }, [allSelected, rows, selected, onSelectedChange]);

  const onRowClick = useCallback(
    (event: MouseEvent<HTMLTableRowElement>, index: number) => {
      const row = rows[index];
      if (event.shiftKey) {
        // Shift always means "extend the selection", never "open" - a range
        // that opened the drawer on its last row would hide half of what was
        // just selected.
        selectRange(index);
        return;
      }
      if (event.metaKey || event.ctrlKey) {
        toggleOne(row.id);
        anchor.current = index;
        return;
      }
      anchor.current = index;
      onOpen(row.id);
    },
    [rows, selectRange, toggleOne, onOpen],
  );

  const onSort = useCallback(
    (key: SortKey) => {
      // Same column flips direction; a new column starts descending, which is
      // the useful end of all three (newest, largest, and a stable A-Z that
      // the second click reverses).
      onSortChange(
        sort.key === key ? { key, desc: !sort.desc } : { key, desc: true },
      );
    },
    [sort, onSortChange],
  );

  /** Category options for the inline cell editor, built once per render. */
  const categoryOptions = useMemo(
    () => [
      { value: "", label: UNCATEGORISED },
      ...categories.map((name) => ({ value: name, label: name })),
    ],
    [categories],
  );

  /** The row and its twins, highlighted together while the popover is open. */
  const twins = useMemo(() => {
    if (!popover) return null;
    return new Set([popover.row.id, ...popover.row.duplicateOf]);
  }, [popover]);

  return (
    <div className="invoice-scroll" ref={scrollRef}>
      <table className="invoice-table">
        <colgroup>
          <col style={{ width: "30px" }} />
          <col style={{ width: "92px" }} />
          <col style={{ width: "158px" }} />
          <col style={{ width: "132px" }} />
          <col />
          <col style={{ width: "124px" }} />
          <col style={{ width: "104px" }} />
          <col style={{ width: "92px" }} />
          <col style={{ width: "112px" }} />
          <col style={{ width: "104px" }} />
        </colgroup>

        <thead>
          <tr>
            <th className="cell-check">
              <input
                ref={headerCheckRef}
                type="checkbox"
                checked={allSelected}
                onChange={onHeaderCheck}
                title="全选当前列表"
              />
            </th>
            <SortableHeader
              label="开票日期"
              sortKey="date"
              sort={sort}
              onSort={onSort}
            />
            <th>发票号码</th>
            <th>票种</th>
            <SortableHeader
              label="销售方名称"
              sortKey="seller"
              sort={sort}
              onSort={onSort}
            />
            <th>费用类别</th>
            <th className="cell-num">金额(不含税)</th>
            <th className="cell-num">税额</th>
            <SortableHeader
              label="价税合计"
              sortKey="amount"
              sort={sort}
              onSort={onSort}
              numeric
            />
            <th>状态</th>
          </tr>
        </thead>

        <tbody>
          {rows.map((row, index) => {
            const duplicate = rowIsDuplicate(row);
            const review = rowNeedsReview(row);
            return (
              <tr
                key={row.id}
                data-row-id={row.id}
                className={[
                  "invoice-row",
                  selected.has(row.id) ? "invoice-row-selected" : "",
                  row.id === activeId ? "invoice-row-active" : "",
                  twins?.has(row.id) ? "invoice-row-twin" : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                onClick={(event) => onRowClick(event, index)}
              >
                <td className="cell-check">
                  <input
                    type="checkbox"
                    checked={selected.has(row.id)}
                    // Selection is decided in onClick rather than onChange
                    // because only the click event carries `shiftKey`, which
                    // is what a range selection is made of. React still wants
                    // a change handler on a controlled checkbox.
                    onChange={() => {}}
                    onClick={(event) => {
                      event.stopPropagation();
                      if (event.shiftKey) {
                        selectRange(index);
                        return;
                      }
                      toggleOne(row.id);
                      anchor.current = index;
                    }}
                  />
                </td>

                <td className="tnum">{formatDate(row.issuedOn)}</td>
                <td className="tnum cell-ellipsis" title={row.number}>
                  {row.number || "—"}
                </td>
                <td className="cell-ellipsis" title={row.kind}>
                  {row.kind || "—"}
                </td>
                <td className="cell-ellipsis" title={row.sellerName}>
                  {row.sellerName || "—"}
                </td>

                <td
                  className="cell-category"
                  // The select swallows its own clicks: re-categorising is the
                  // most repeated action in the app and it must not cost a
                  // detour through the drawer.
                  onClick={(event) => event.stopPropagation()}
                  title={
                    row.categoryRule
                      ? `由规则「${row.categoryRule}」判定`
                      : "手动设置"
                  }
                >
                  <Select
                    value={row.category}
                    options={
                      // A category assigned by a rule that no longer exists
                      // still has to be selectable, or opening the select
                      // would silently rewrite the row's category to whatever
                      // happens to be first.
                      row.category && !categories.includes(row.category)
                        ? [
                            ...categoryOptions,
                            { value: row.category, label: row.category },
                          ]
                        : categoryOptions
                    }
                    onChange={(value) => onCategoryChange(row.id, value)}
                  />
                </td>

                <td className="cell-num tnum">
                  {formatAmount(amountExclTaxOf(row))}
                </td>
                <td className="cell-num tnum">{formatAmount(row.taxCents)}</td>
                <td className="cell-num tnum cell-total">
                  {formatAmount(row.totalCents)}
                </td>

                <td className="cell-status">
                  {duplicate && (
                    <button
                      className="status-badge-button"
                      title="点击查看重复的另一张"
                      onClick={(event) => {
                        event.stopPropagation();
                        const rect =
                          event.currentTarget.getBoundingClientRect();
                        setPopover(
                          popover?.row.id === row.id
                            ? null
                            : { row, left: rect.left, top: rect.bottom + 4 },
                        );
                      }}
                    >
                      <Badge tone="danger">疑似重复</Badge>
                    </button>
                  )}
                  {review && <Badge tone="warn">待复核</Badge>}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {popover && (
        <div
          ref={popoverRef}
          className="dup-popover"
          style={{ left: popover.left, top: popover.top }}
          onClick={(event) => event.stopPropagation()}
        >
          <div className="dup-popover-title">
            同一张发票号码还出现在 {popover.row.duplicateOf.length} 处
          </div>
          {popover.row.duplicateOf.map((id) => {
            const twin = duplicateIndex.get(id);
            return (
              <button
                key={id}
                className="dup-peer"
                onClick={() => {
                  // Opening the twin rather than scrolling to it: the drawer
                  // works by id, so this behaves the same whether or not the
                  // current filter happens to include the other row - and the
                  // table scrolls to it by itself when it does.
                  setPopover(null);
                  onOpen(id);
                }}
              >
                <span className="dup-peer-date tnum">
                  {twin ? formatDate(twin.issuedOn) : "—"}
                </span>
                <span className="dup-peer-seller">
                  {twin?.sellerName || `#${id}`}
                </span>
                <span className="dup-peer-amount tnum">
                  {twin ? formatMoney(twin.totalCents) : ""}
                </span>
              </button>
            );
          })}
          <div className="dup-popover-hint">
            重复不一定是错的（同一张票可能补开），但只应报销一次。
          </div>
        </div>
      )}
    </div>
  );
}

function SortableHeader({
  label,
  sortKey,
  sort,
  onSort,
  numeric,
}: {
  label: string;
  sortKey: SortKey;
  sort: Sort;
  onSort: (key: SortKey) => void;
  numeric?: boolean;
}) {
  const active = sort.key === sortKey;
  return (
    <th className={numeric ? "cell-num" : undefined}>
      <button
        className={`th-sort ${active ? "th-sort-active" : ""}`}
        onClick={() => onSort(sortKey)}
      >
        {label}
        <ChevronIcon
          className={`th-sort-icon ${active && sort.desc ? "th-sort-icon-desc" : ""} ${
            active ? "" : "th-sort-icon-idle"
          }`}
        />
      </button>
    </th>
  );
}
