import { useCallback, useEffect, useMemo, useState } from "react";
import { ImportOverlay, useImport } from "./ingest/useImport";
import { useAppUpdate } from "./utils/useAppUpdate";
import { InvoicesPane } from "./invoices/InvoicesPane";
import { ReportsPane } from "./reports/ReportsPane";
import { RulesPane } from "./rules/RulesPane";
import { SettingsPane } from "./settings/SettingsPane";
import { formatMoney } from "./format";
import { invoices as invoicesApi } from "./ipc";
import {
  DuplicateIcon,
  ImportIcon,
  InvoiceIcon,
  ReportIcon,
  ReviewIcon,
  RulesIcon,
  SettingsIcon,
} from "./ui/icons";
import { Button, ToastProvider, useToast } from "./ui/primitives";
import { WindowControls } from "./ui/WindowControls";
import "./App.css";

/**
 * Which pane is on screen.
 *
 * The three invoice scopes are one pane with a filter rather than three
 * panes, because they are the same table answering "what do I have", "what
 * needs me", and "what might already be claimed" - and a user moves between
 * those constantly while reviewing a month's expenses.
 */
export type View =
  | { kind: "invoices"; scope: "all" | "review" | "duplicates" }
  | { kind: "reports" }
  | { kind: "rules" }
  | { kind: "settings" };

/** Counts for the sidebar badges. */
interface Counts {
  all: number;
  review: number;
  duplicates: number;
  totalCents: number;
}

function Shell() {
  const [view, setView] = useState<View>({ kind: "invoices", scope: "all" });
  const [counts, setCounts] = useState<Counts>({
    all: 0,
    review: 0,
    duplicates: 0,
    totalCents: 0,
  });
  const toast = useToast();

  const refreshCounts = useCallback(async () => {
    try {
      const [all, review, duplicates] = await Promise.all([
        invoicesApi.list({}),
        invoicesApi.list({ needsReviewOnly: true }),
        invoicesApi.list({ duplicatesOnly: true }),
      ]);
      setCounts({
        all: all.length,
        review: review.length,
        duplicates: duplicates.length,
        totalCents: all.reduce((sum, row) => sum + row.totalCents, 0),
      });
    } catch (error) {
      toast(String(error), "error");
    }
  }, [toast]);

  // The import hook owns drag-and-drop for the whole window, so a file can be
  // dropped from any pane - which is how people actually use it: they are
  // looking at last month's list when the new invoices arrive by mail.
  const importer = useImport(refreshCounts);

  // Checked on launch and every few hours. The banner below is the only thing
  // it can put on screen, and only when there is genuinely a newer build.
  const appUpdate = useAppUpdate();

  useEffect(() => {
    void refreshCounts();
  }, [refreshCounts]);

  // The import summary offers 「去设置里开启」 when a photo could not be read
  // and the vision fallback is off. It asks for that navigation by dispatching
  // an event rather than being handed a callback, because the overlay is
  // mounted by the shell but rendered over whichever pane is active - and it
  // has no business knowing what the shell's views are called. Claiming the
  // event with preventDefault() is what tells the overlay the request landed;
  // unclaimed, it falls back to telling the user where the setting lives.
  useEffect(() => {
    const onOpenSettings = (event: Event) => {
      event.preventDefault();
      setView({ kind: "settings" });
    };
    window.addEventListener("invoicedesk:open-settings", onOpenSettings);
    return () =>
      window.removeEventListener("invoicedesk:open-settings", onOpenSettings);
  }, []);

  const sidebar = useMemo(
    () => [
      {
        group: "发票",
        items: [
          {
            key: "all",
            label: "全部发票",
            icon: <InvoiceIcon />,
            count: counts.all,
            tone: "" as const,
            view: { kind: "invoices", scope: "all" } as View,
          },
          {
            key: "review",
            label: "待复核",
            icon: <ReviewIcon />,
            count: counts.review,
            tone: "warn" as const,
            view: { kind: "invoices", scope: "review" } as View,
          },
          {
            key: "duplicates",
            label: "疑似重复",
            icon: <DuplicateIcon />,
            count: counts.duplicates,
            tone: "danger" as const,
            view: { kind: "invoices", scope: "duplicates" } as View,
          },
        ],
      },
      {
        group: "整理",
        items: [
          {
            key: "reports",
            label: "报销单",
            icon: <ReportIcon />,
            count: null,
            tone: "" as const,
            view: { kind: "reports" } as View,
          },
          {
            key: "rules",
            label: "分类规则",
            icon: <RulesIcon />,
            count: null,
            tone: "" as const,
            view: { kind: "rules" } as View,
          },
          {
            key: "settings",
            label: "设置",
            icon: <SettingsIcon />,
            count: null,
            tone: "" as const,
            view: { kind: "settings" } as View,
          },
        ],
      },
    ],
    [counts],
  );

  const activeKey = view.kind === "invoices" ? view.scope : view.kind;

  return (
    <div className="app-shell">
      <div className="window-bar" data-tauri-drag-region>
        <span className="window-bar-title">智票</span>
        <span className="window-bar-spacer" />
        {counts.all > 0 && (
          <span className="window-bar-stat">
            共 <strong>{counts.all}</strong> 张 · 合计{" "}
            <strong>{formatMoney(counts.totalCents)}</strong>
          </span>
        )}
        <WindowControls />
      </div>

      <div className="app-body">
        <nav className="sidebar">
          <Button intent="primary" onClick={importer.pickFiles}>
            <ImportIcon />
            导入发票
          </Button>

          {sidebar.map((section) => (
            <div key={section.group}>
              <div className="sidebar-group-title">{section.group}</div>
              {section.items.map((item) => (
                <button
                  key={item.key}
                  className={`sidebar-item ${
                    activeKey === item.key ? "sidebar-item-active" : ""
                  }`}
                  onClick={() => setView(item.view)}
                >
                  {item.icon}
                  <span className="sidebar-item-label">{item.label}</span>
                  {item.count !== null && item.count > 0 && (
                    <span
                      className={`sidebar-count ${
                        item.tone ? `sidebar-count-${item.tone}` : ""
                      }`}
                    >
                      {item.count}
                    </span>
                  )}
                </button>
              ))}
            </div>
          ))}
        </nav>

        <main className="main">
          {view.kind === "invoices" && (
            <InvoicesPane scope={view.scope} onChanged={refreshCounts} />
          )}
          {view.kind === "reports" && <ReportsPane />}
          {view.kind === "rules" && <RulesPane onChanged={refreshCounts} />}
          {view.kind === "settings" && <SettingsPane />}
        </main>
      </div>

      {appUpdate.status !== "idle" && appUpdate.status !== "checking" && (
        <UpdateBanner update={appUpdate} />
      )}

      <ImportOverlay importer={importer} />
    </div>
  );
}

/**
 * The update prompt.
 *
 * A strip along the bottom rather than a modal: nothing about a new version
 * is urgent enough to interrupt someone mid-way through reviewing a month of
 * invoices, and a dialog they have to dismiss to keep working is exactly how
 * an update prompt trains people to click "later" without reading.
 */
function UpdateBanner({ update }: { update: ReturnType<typeof useAppUpdate> }) {
  const downloading = update.status === "downloading";
  return (
    <div className="update-banner">
      <span className="update-banner-text">
        {update.status === "error"
          ? `更新失败：${update.error ?? "未知错误"}`
          : `有新版本 ${update.version}`}
      </span>
      {update.status !== "error" && (
        <Button
          intent="primary"
          onClick={() => void update.install()}
          disabled={downloading}
        >
          {downloading ? "下载中…" : "更新并重启"}
        </Button>
      )}
      <Button onClick={update.dismiss} disabled={downloading}>
        {update.status === "error" ? "知道了" : "以后再说"}
      </Button>
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}
