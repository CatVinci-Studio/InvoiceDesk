/**
 * Assembling one 报销单: its header, the invoices on it, and the preview that
 * has to be read before anything is exported.
 *
 * Three stacked sections, in the order the work actually happens:
 *
 *   1. 表头  - who is claiming, for what, dated when
 *   2. 选择发票 - candidates on the left, the sheet on the right
 *   3. 核对与导出 - the totals, 合计大写, 分类汇总, and the problems
 *
 * The third section is the reason the pane exists. Exporting is one click
 * from anywhere in this app; noticing that one of the twenty invoices on the
 * sheet was already claimed last month is not. So the export button lives at
 * the very bottom, underneath the duplicate and validation blocks, and a
 * person on their way to it has to scroll past both.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  countLabel,
  formatAmount,
  formatDate,
  formatMoney,
  monthsAgo,
  today,
} from "../format";
import {
  errorMessage,
  invoices as invoicesApi,
  prefs,
  reports as reportsApi,
  rules as rulesApi,
} from "../ipc";
import type { InvoiceRow, Report, ReportMeta, ReportPreview } from "../types";
import { ChevronIcon, CloseIcon, PlusIcon, ReportIcon } from "../ui/icons";
import {
  Badge,
  Button,
  Group,
  Row,
  SearchInput,
  Select,
  TextInput,
  useAsync,
  useToast,
} from "../ui/primitives";
import { ExportDialog } from "./ExportDialog";
import {
  EMPTY_PREVIEW,
  formatShare,
  moveRow,
  rowFromInvoice,
  rowFromListRow,
  shareOf,
  sortByDate,
  sumCents,
  type PickedRow,
} from "./rows";

/**
 * Where a sheet's 制表日期 is kept.
 *
 * `ReportMeta` carries a date but the `reports` table does not have a column
 * for one (see `db::save_report`), and the date belongs to the sheet rather
 * than to the app - a sheet written on the 3rd and re-exported on the 5th
 * must not silently change the date finance already saw. So it goes in the
 * generic preference store under the report's id. If a date column is ever
 * added to `reports`, this and its two call sites are what to delete.
 */
function dateKey(reportId: number): string {
  return `report.${reportId}.date`;
}

/**
 * A native date field, styled as one of ours.
 *
 * `primitives.tsx` has no date input and is not mine to extend, so this
 * borrows the shared `.input` class rather than inventing a look. The native
 * control is worth it here: it renders in the user's locale, it enforces
 * `YYYY-MM-DD` on the way out, and that is exactly the format both the filter
 * and `ReportMeta.date` want.
 */
function DateInput({
  value,
  onChange,
  title,
}: {
  value: string;
  onChange: (value: string) => void;
  title?: string;
}) {
  return (
    <input
      type="date"
      className="input rp-date"
      value={value}
      title={title}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

export function ReportBuilder({
  report,
  onSaved,
}: {
  report: Report;
  onSaved: () => void;
}) {
  const toast = useToast();
  const reportId = report.id ?? 0;

  // --- 表头 ---------------------------------------------------------------
  const [meta, setMeta] = useState<ReportMeta>({
    title: report.title,
    applicant: report.applicant,
    department: report.department,
    note: report.note,
    date: today(),
  });

  useEffect(() => {
    let alive = true;
    prefs
      .get(dateKey(reportId))
      .then((stored) => {
        if (alive && stored)
          setMeta((current) => ({ ...current, date: stored }));
      })
      // A missing preference is the normal case for a new sheet, and today's
      // date is already in state - nothing to report.
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [reportId]);

  const persistMeta = useCallback(
    async (next: ReportMeta) => {
      try {
        await reportsApi.save({
          ...report,
          title: next.title,
          applicant: next.applicant,
          department: next.department,
          note: next.note,
        });
        await prefs.set(dateKey(reportId), next.date);
        onSaved();
      } catch (error) {
        toast(errorMessage(error), "error");
      }
    },
    [onSaved, report, reportId, toast],
  );

  const editMeta = (key: keyof ReportMeta) => (value: string) =>
    setMeta((current) => ({ ...current, [key]: value }));

  // --- 单上的发票 ---------------------------------------------------------
  // Held locally rather than read straight off `reports.invoices` so a
  // reorder repaints on the click instead of after the round trip. The server
  // remains the authority: every mutation writes the whole ordered list back
  // and puts the previous order back on the screen if the write fails.
  const [picked, setPicked] = useState<PickedRow[]>([]);
  const [pickedLoaded, setPickedLoaded] = useState(false);
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let alive = true;
    reportsApi
      .invoices(reportId)
      .then((list) => {
        if (!alive) return;
        setPicked(list.map(rowFromInvoice));
        setPickedLoaded(true);
      })
      .catch((error: unknown) => {
        if (!alive) return;
        // Marked loaded anyway: an empty list with a toast beside it is
        // honest, whereas "正在读取…" forever is not.
        setPickedLoaded(true);
        toast(errorMessage(error), "error");
      });
    return () => {
      alive = false;
    };
  }, [reportId, toast]);

  const commit = useCallback(
    async (next: PickedRow[]) => {
      const previous = picked;
      setPicked(next);
      try {
        // `setInvoices` replaces the whole list and stores the array order as
        // the row order, so the full ordered set goes over every time - there
        // is no incremental add or move on the other side.
        await reportsApi.setInvoices(
          reportId,
          next.map((row) => row.id),
        );
        setVersion((current) => current + 1);
        onSaved();
      } catch (error) {
        setPicked(previous);
        toast(errorMessage(error), "error");
      }
    },
    [onSaved, picked, reportId, toast],
  );

  // --- 候选发票 -----------------------------------------------------------
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [checked, setChecked] = useState<ReadonlySet<number>>(
    new Set<number>(),
  );

  const { data: candidates, loading: candidatesLoading } = useAsync<
    InvoiceRow[]
  >(
    () =>
      invoicesApi.list({
        search: search || null,
        category: category || null,
        from: from || null,
        to: to || null,
        // The whole reason the picker never shows an invoice twice: the
        // filter itself excludes anything already on this sheet.
        excludeReport: reportId,
      }),
    [reportId, search, category, from, to, version],
    [],
  );

  const { data: categories } = useAsync<string[]>(
    () => rulesApi.categories(),
    [],
    [],
  );

  const toggleCandidate = useCallback((id: number) => {
    setChecked((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  // Only the rows currently on screen count: a filter change must not leave
  // the button promising to add invoices the user can no longer see.
  const checkedVisible = useMemo(
    () => candidates.filter((row) => checked.has(row.id)),
    [candidates, checked],
  );

  const addChecked = useCallback(() => {
    if (checkedVisible.length === 0) return;
    void commit([...picked, ...checkedVisible.map(rowFromListRow)]);
    setChecked(new Set<number>());
  }, [checkedVisible, commit, picked]);

  const addAll = useCallback(() => {
    if (candidates.length === 0) return;
    void commit([...picked, ...candidates.map(rowFromListRow)]);
    setChecked(new Set<number>());
  }, [candidates, commit, picked]);

  // --- 预览 ---------------------------------------------------------------
  const { data: preview, loading: previewLoading } = useAsync<ReportPreview>(
    () => reportsApi.preview(reportId),
    [reportId, version],
    EMPTY_PREVIEW,
  );

  // The duplicate check runs over the whole ledger on the Rust side and comes
  // back as invoice numbers; matching them onto the rows puts the warning on
  // the offending line as well as in the block at the bottom.
  const duplicateNumbers = useMemo(
    () => new Set(preview.duplicates),
    [preview.duplicates],
  );

  const [exporting, setExporting] = useState(false);
  const pickedTotal = sumCents(picked);

  return (
    <div className="rp-builder">
      <div className="rp-builder-scroll">
        {/* --- 1. 表头 --- */}
        <Group
          title="表头信息"
          hint="这几项会写进导出的表头，模板导出时对应 {{标题}} {{申请人}} {{部门}} {{日期}} {{说明}}。"
        >
          <Row label="标题">
            <TextInput
              value={meta.title}
              onChange={editMeta("title")}
              onBlur={() => void persistMeta(meta)}
              placeholder="2026年8月报销单"
            />
          </Row>
          <Row label="申请人">
            <TextInput
              value={meta.applicant}
              onChange={editMeta("applicant")}
              onBlur={() => void persistMeta(meta)}
            />
          </Row>
          <Row label="部门">
            <TextInput
              value={meta.department}
              onChange={editMeta("department")}
              onBlur={() => void persistMeta(meta)}
            />
          </Row>
          <Row label="制表日期" hint="默认今天">
            <DateInput
              value={meta.date}
              onChange={(value) => {
                const next = { ...meta, date: value };
                setMeta(next);
                // A date picker closes without blurring the field, so this one
                // saves on change rather than on blur.
                void persistMeta(next);
              }}
            />
          </Row>
          <Row label="说明" stacked>
            <TextInput
              value={meta.note}
              onChange={editMeta("note")}
              onBlur={() => void persistMeta(meta)}
              placeholder="例如：8月上海出差，含高铁、住宿与餐饮"
            />
          </Row>
        </Group>

        {/* --- 2. 选择发票 --- */}
        <section className="rp-section">
          <h3 className="rp-section-title">选择发票</h3>
          <div className="rp-picker">
            <div className="rp-panel">
              <div className="rp-panel-head">
                <span>候选发票</span>
                <span className="rp-panel-spacer" />
                <span className="tnum rp-panel-count">{candidates.length}</span>
              </div>

              <div className="rp-filters">
                <SearchInput value={search} onChange={setSearch} />
                <Select
                  value={category}
                  onChange={setCategory}
                  options={[
                    { value: "", label: "全部类别" },
                    ...categories.map((name) => ({ value: name, label: name })),
                  ]}
                />
                <div className="rp-filter-dates">
                  <DateInput
                    value={from}
                    onChange={setFrom}
                    title="开票日期起"
                  />
                  <span className="rp-filter-dash">–</span>
                  <DateInput value={to} onChange={setTo} title="开票日期止" />
                </div>
                {/* Two presets rather than a range picker: assembling a sheet
                    is almost always "this month" or "the last quarter", and
                    those two cover it without another popover. */}
                <Button
                  onClick={() => {
                    setFrom(monthsAgo(0));
                    setTo("");
                  }}
                >
                  本月
                </Button>
                <Button
                  onClick={() => {
                    setFrom(monthsAgo(2));
                    setTo("");
                  }}
                >
                  近三个月
                </Button>
                {(search || category || from || to) && (
                  <Button
                    onClick={() => {
                      setSearch("");
                      setCategory("");
                      setFrom("");
                      setTo("");
                    }}
                  >
                    清除筛选
                  </Button>
                )}
              </div>

              <div className="rp-list">
                {candidatesLoading && candidates.length === 0 && (
                  <p className="rp-list-empty">正在读取…</p>
                )}
                {!candidatesLoading && candidates.length === 0 && (
                  <p className="rp-list-empty">
                    没有符合条件的发票。已经在这张单上的发票不会出现在这里。
                  </p>
                )}
                {candidates.map((row) => {
                  const item = rowFromListRow(row);
                  return (
                    <label
                      key={item.id}
                      className={`rp-row rp-row-candidate ${
                        checked.has(item.id) ? "rp-row-checked" : ""
                      }`}
                    >
                      <input
                        type="checkbox"
                        className="rp-check"
                        checked={checked.has(item.id)}
                        onChange={() => toggleCandidate(item.id)}
                      />
                      <RowBody row={item} />
                    </label>
                  );
                })}
              </div>

              <div className="rp-panel-foot">
                <Button
                  intent="primary"
                  disabled={checkedVisible.length === 0}
                  onClick={addChecked}
                >
                  <PlusIcon />
                  加入报销单
                  {checkedVisible.length > 0
                    ? `（${checkedVisible.length}）`
                    : ""}
                </Button>
                <Button disabled={candidates.length === 0} onClick={addAll}>
                  全部加入
                </Button>
                <span className="rp-panel-spacer" />
                {checkedVisible.length > 0 && (
                  <span className="tnum">
                    {formatMoney(
                      checkedVisible.reduce(
                        (sum, row) => sum + row.totalCents,
                        0,
                      ),
                    )}
                  </span>
                )}
              </div>
            </div>

            <div className="rp-panel">
              <div className="rp-panel-head">
                <span>已在这张单上</span>
                <span className="rp-panel-spacer" />
                <Button
                  disabled={picked.length < 2}
                  title="按开票日期从早到晚排序"
                  onClick={() => void commit(sortByDate(picked))}
                >
                  按日期排序
                </Button>
              </div>

              <div className="rp-list">
                {!pickedLoaded && <p className="rp-list-empty">正在读取…</p>}
                {pickedLoaded && picked.length === 0 && (
                  <p className="rp-list-empty">
                    还没有发票。从左边勾选后加入，顺序就是明细表里的行序。
                  </p>
                )}
                {picked.map((row, index) => (
                  <div key={row.id} className="rp-row rp-row-picked">
                    <span className="rp-index tnum">{index + 1}</span>
                    <RowBody
                      row={{
                        ...row,
                        duplicate:
                          row.duplicate || duplicateNumbers.has(row.number),
                      }}
                    />
                    <span className="rp-row-actions">
                      <button
                        className="rp-icon-btn"
                        title="上移"
                        disabled={index === 0}
                        onClick={() => void commit(moveRow(picked, index, -1))}
                      >
                        <ChevronIcon className="rp-chevron-up" />
                      </button>
                      <button
                        className="rp-icon-btn"
                        title="下移"
                        disabled={index === picked.length - 1}
                        onClick={() => void commit(moveRow(picked, index, 1))}
                      >
                        <ChevronIcon className="rp-chevron-down" />
                      </button>
                      <button
                        className="rp-icon-btn rp-icon-btn-danger"
                        title="从报销单移出（不会删除发票）"
                        onClick={() =>
                          void commit(
                            picked.filter((other) => other.id !== row.id),
                          )
                        }
                      >
                        <CloseIcon />
                      </button>
                    </span>
                  </div>
                ))}
              </div>

              {/* The running total, in the place a spreadsheet puts it. */}
              <div className="rp-panel-foot rp-panel-foot-total">
                <span>共 {countLabel(picked.length)}</span>
                <span className="rp-panel-spacer" />
                <span>合计</span>
                <strong className="tnum">{formatMoney(pickedTotal)}</strong>
              </div>
            </div>
          </div>
        </section>

        {/* --- 3. 核对与导出 --- */}
        <section className="rp-section">
          <h3 className="rp-section-title">核对与导出</h3>

          <div className="rp-figures">
            <Figure label="张数" value={String(preview.count)} />
            <Figure
              label="金额（不含税）"
              value={formatAmount(preview.amountExclTaxCents)}
            />
            <Figure label="税额" value={formatAmount(preview.taxCents)} />
            <Figure
              label="价税合计"
              value={formatAmount(preview.totalCents)}
              strong
            />
            <Figure
              label="其中可抵扣进项税额"
              value={formatAmount(preview.deductibleTaxCents)}
              hint="只统计专用发票"
            />
          </div>

          {/* 合计大写 gets its own line at full size: it is the field the
              person approving the sheet reads first, and the one that has to
              be checked against the figures above it by eye. */}
          <div className="rp-capital">
            <span className="rp-capital-label">合计金额（大写）</span>
            <span className="rp-capital-value">{preview.capitalAmount}</span>
          </div>

          {preview.byCategory.length > 0 && (
            <table className="rp-table">
              <thead>
                <tr>
                  <th>费用类别</th>
                  <th className="rp-num">张数</th>
                  <th className="rp-num">价税合计</th>
                  <th className="rp-num">占比</th>
                </tr>
              </thead>
              <tbody>
                {preview.byCategory.map((line) => {
                  const share = shareOf(line.totalCents, preview.totalCents);
                  return (
                    <tr key={line.category}>
                      <td>{line.category}</td>
                      <td className="rp-num tnum">{line.count}</td>
                      <td className="rp-num tnum">
                        {formatAmount(line.totalCents)}
                      </td>
                      <td className="rp-num">
                        <span className="rp-share">
                          <span className="rp-share-track">
                            <span
                              className="rp-share-fill"
                              style={{ width: `${Math.round(share * 100)}%` }}
                            />
                          </span>
                          <span className="tnum">{formatShare(share)}</span>
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}

          {/* The most severe thing this app can tell anyone. Not a hard block
              - the same invoice number legitimately appears twice often
              enough (a re-issued 红字发票, a scan imported from two folders) -
              but it is the sentence that saves someone from claiming an
              expense twice, so it goes first and it says what it means. */}
          {preview.duplicates.length > 0 && (
            <div className="rp-alert rp-alert-danger">
              <div className="rp-alert-title">
                疑似重复报销（{preview.duplicates.length} 张）
              </div>
              <p className="rp-alert-body">
                下面这些发票号码在发票库里还有另一张同号的发票，很可能已经报过了。
                导出前请逐张确认，确实是两张不同的票再继续。
              </p>
              <div className="rp-chips">
                {preview.duplicates.map((number) => (
                  <span key={number} className="rp-chip tnum">
                    {number}
                  </span>
                ))}
              </div>
            </div>
          )}

          {preview.warnings.length > 0 && (
            <div className="rp-alert rp-alert-warn">
              <div className="rp-alert-title">
                校验问题（{preview.warnings.length} 条）
              </div>
              <p className="rp-alert-body">
                这些问题会以红字写进导出的明细表。价税合计对不上的发票，建议先回发票列表改好再导出。
              </p>
              <ul className="rp-alert-list">
                {preview.warnings.map(([number, problem], index) => (
                  <li key={`${number}-${index}`}>
                    <span className="tnum rp-alert-number">{number}</span>
                    {problem}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <div className="rp-export">
            <span className="rp-export-hint">
              {preview.count === 0
                ? "报销单还是空的，先加入发票。"
                : previewLoading
                  ? "正在核对…"
                  : `导出 ${countLabel(preview.count)}，合计 ${formatMoney(preview.totalCents)}。`}
            </span>
            <Button
              intent="primary"
              disabled={preview.count === 0}
              onClick={() => setExporting(true)}
            >
              <ReportIcon />
              导出 Excel…
            </Button>
          </div>
        </section>
      </div>

      {exporting && (
        <ExportDialog
          reportId={reportId}
          meta={meta}
          invoiceCount={preview.count}
          duplicateCount={preview.duplicates.length}
          onClose={() => setExporting(false)}
        />
      )}
    </div>
  );
}

/** One invoice, rendered identically on both sides of the picker. */
function RowBody({ row }: { row: PickedRow }) {
  return (
    <span className="rp-row-body">
      <span className="rp-row-title">
        <span className="rp-row-name">
          {row.seller || row.kind || "未知销售方"}
        </span>
        <span className="rp-row-amount tnum">
          {formatAmount(row.totalCents)}
        </span>
      </span>
      <span className="rp-row-meta">
        <span className="tnum">{formatDate(row.issuedOn)}</span>
        <span className="tnum rp-row-number">{row.number}</span>
        <span className="rp-row-category">{row.category}</span>
        {row.duplicate && <Badge tone="danger">疑似重复</Badge>}
        {row.issueCount > 0 && (
          <Badge tone="warn">{row.issueCount} 个问题</Badge>
        )}
      </span>
    </span>
  );
}

function Figure({
  label,
  value,
  hint,
  strong,
}: {
  label: string;
  value: string;
  hint?: string;
  strong?: boolean;
}) {
  return (
    <div
      className={`rp-figure ${strong ? "rp-figure-strong" : ""}`}
      title={hint}
    >
      <span className="rp-figure-label">{label}</span>
      <span className="rp-figure-value tnum">{value}</span>
    </div>
  );
}
