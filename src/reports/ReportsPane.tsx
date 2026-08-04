/**
 * 报销单 - the pane the whole app builds towards.
 *
 * Everything before this point is bookkeeping; this is where a set of
 * invoices becomes the document handed to 财务. The pane is a two-column
 * shell: the saved sheets on the left, and the one being worked on filling
 * the rest of the window.
 *
 * The list is on the LEFT rather than being a dropdown over a single editor
 * because a user has several sheets alive at once in the days around a
 * month's close - last month's, awaiting approval, and this month's, still
 * being assembled - and moving between them has to cost one click and no
 * memory of what they were called.
 */

import { useCallback, useState } from "react";
import { countLabel, formatDate, formatMoney } from "../format";
import { errorMessage, reports as reportsApi } from "../ipc";
import type { Report } from "../types";
import { PlusIcon, ReportIcon, TrashIcon } from "../ui/icons";
import { Button, Modal, useAsync, useToast } from "../ui/primitives";
import { ReportBuilder } from "./ReportBuilder";
import { defaultReportTitle } from "./rows";
import "./reports.css";

/** `2026-08-03T11:02:41+08:00` → `2026/08/03`. Stored as RFC 3339 by
 *  `db::save_report`; only the date half is worth a line this narrow. */
function createdLabel(createdAt: string): string {
  return formatDate(createdAt.slice(0, 10));
}

export function ReportsPane() {
  const toast = useToast();
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [pendingDelete, setPendingDelete] = useState<Report | null>(null);

  const {
    data: allReports,
    loading,
    reload,
  } = useAsync<Report[]>(() => reportsApi.list(), [], []);

  // Falling back to the newest sheet rather than showing an empty right-hand
  // side means arriving at this pane always lands on something - and after a
  // delete, on the next sheet rather than on nothing.
  const selected =
    allReports.find((report) => report.id === selectedId) ??
    allReports[0] ??
    null;

  const create = useCallback(async () => {
    try {
      // 申请人 and 部门 carry over from the most recent sheet. It is the same
      // person filing every month, and retyping their own name and department
      // twelve times a year is exactly the kind of friction this app exists
      // to remove.
      const previous = allReports[0];
      const id = await reportsApi.save({
        id: null,
        title: defaultReportTitle(),
        applicant: previous?.applicant ?? "",
        department: previous?.department ?? "",
        note: "",
        createdAt: "",
        invoiceCount: 0,
        totalCents: 0,
      });
      setSelectedId(id);
      reload();
    } catch (error) {
      toast(errorMessage(error), "error");
    }
  }, [allReports, reload, toast]);

  const confirmDelete = useCallback(async () => {
    const target = pendingDelete;
    if (!target?.id) return;
    try {
      await reportsApi.remove(target.id);
      setPendingDelete(null);
      if (selectedId === target.id) setSelectedId(null);
      reload();
      toast("报销单已删除，单上的发票仍在发票库里");
    } catch (error) {
      toast(errorMessage(error), "error");
    }
  }, [pendingDelete, reload, selectedId, toast]);

  return (
    <div className="rp">
      <aside className="rp-sidebar">
        <div className="rp-sidebar-head">
          <span className="rp-sidebar-title">报销单</span>
          <Button intent="primary" onClick={() => void create()}>
            <PlusIcon />
            新建
          </Button>
        </div>

        <div className="rp-sidebar-list">
          {!loading && allReports.length === 0 && (
            <p className="rp-sidebar-empty">还没有报销单</p>
          )}
          {allReports.map((report) => (
            <div
              key={report.id}
              className={`rp-report ${
                selected?.id === report.id ? "rp-report-active" : ""
              }`}
            >
              <button
                className="rp-report-open"
                onClick={() => setSelectedId(report.id)}
              >
                <span className="rp-report-title">
                  {report.title || "未命名报销单"}
                </span>
                <span className="rp-report-meta">
                  <span className="tnum">
                    {countLabel(report.invoiceCount)}
                  </span>
                  <span className="rp-report-amount tnum">
                    {formatMoney(report.totalCents)}
                  </span>
                </span>
                <span className="rp-report-created">
                  {createdLabel(report.createdAt)}
                </span>
              </button>
              <button
                className="rp-report-delete"
                title="删除报销单"
                onClick={() => setPendingDelete(report)}
              >
                <TrashIcon />
              </button>
            </div>
          ))}
        </div>
      </aside>

      <div className="rp-main">
        {selected?.id ? (
          // Keyed by id so switching sheets remounts the builder: the header
          // fields, the picker's filters and its selection are all per-sheet
          // state, and carrying any of it across would be worse than useless.
          <ReportBuilder key={selected.id} report={selected} onSaved={reload} />
        ) : (
          <div className="empty">
            <ReportIcon />
            <div className="empty-title">还没有报销单</div>
            <p className="empty-hint">
              新建一张，把这个月要报的发票挑进来，核对合计和大写金额，最后导出成
              Excel 交给财务。
            </p>
            <Button intent="primary" onClick={() => void create()}>
              <PlusIcon />
              新建报销单
            </Button>
          </div>
        )}
      </div>

      {pendingDelete && (
        <Modal
          title="删除报销单"
          onClose={() => setPendingDelete(null)}
          footer={
            <>
              <Button onClick={() => setPendingDelete(null)}>取消</Button>
              <Button intent="danger" onClick={() => void confirmDelete()}>
                删除
              </Button>
            </>
          }
        >
          <p className="rp-confirm">
            确定删除「{pendingDelete.title || "未命名报销单"}」？
          </p>
          {/* The one thing a person hesitating over this button needs to know.
              Said plainly, and said before the click rather than in a toast
              after it. */}
          <p className="rp-confirm-hint">
            只删除这张报销单。单上的 {countLabel(pendingDelete.invoiceCount)}
            发票会留在发票库里，可以放进别的报销单。
          </p>
        </Modal>
      )}
    </div>
  );
}
