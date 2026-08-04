/**
 * 分类规则 - the rule set that decides which 报销类别 an invoice lands in.
 *
 * The design brief for this pane is one sentence: a user should be able to
 * answer 「为什么这张发票被归到餐饮？」 by reading it, without clicking
 * anything. That is why every rule is rendered as a sentence rather than as
 * the条件 JSON it is underneath, why the groups are ordered the way
 * `classify()` evaluates them, and why the priority number is shown next to
 * the band it belongs to instead of on its own.
 *
 * See `src-tauri/src/classify/mod.rs` for why the app classifies with rules at
 * all: the same twenty vendors recur every month, and a rule that was right in
 * March is right in November - for free, offline, and identically every time.
 * This pane is where that asset accumulates, which is also why it can be
 * exported.
 */

import { useCallback, useMemo, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { MATCH_FIELD_LABEL, type Rule } from "../types";
import {
  errorMessage,
  invoices as invoicesApi,
  rules as rulesApi,
} from "../ipc";
import { PlusIcon, RefreshIcon, TrashIcon } from "../ui/icons";
import {
  Badge,
  Button,
  Group,
  Row,
  Toggle,
  useAsync,
  useToast,
} from "../ui/primitives";
import { RuleEditor } from "./RuleEditor";
import {
  CONDITION_JOINER,
  CONTAINS,
  KEYWORD_JOINER,
  PRIORITY_BANDS,
  blankRule,
  categoriesOf,
  describeRule,
  groupByCategory,
  isAutoName,
} from "./ruleText";
import "./rules.css";

/**
 * The file the rule set travels in.
 *
 * A rule set is the thing a user builds up over months, so it has to be a
 * file they can put in a shared drive or mail to a colleague - not something
 * that only exists inside the app's database. The frontend picks the path and
 * the backend does the reading and writing (`export_rules_to_file` /
 * `import_rules_from_file`), which is the same split the report export uses
 * and the reason the app needs no filesystem permission of its own.
 */
const RULES_FILE_FILTER = [{ name: "规则文件", extensions: ["json"] }];
const DEFAULT_RULES_FILENAME = "invoicedesk-规则.json";

export function RulesPane({ onChanged }: { onChanged: () => void }) {
  const toast = useToast();
  const query = useAsync<Rule[]>(() => rulesApi.list(), [], []);
  const { data: rules, loading, error, reload } = query;

  const [editing, setEditing] = useState<Rule | null>(null);
  const [confirming, setConfirming] = useState<number | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  /**
   * Reload the list and tell the shell something moved.
   *
   * Strictly, only 重新分类 can change an invoice's category - saving a rule
   * changes nothing until the rules are re-run. `onChanged` is three cheap
   * queries though, and a sidebar that disagrees with the ledger is a much
   * more expensive kind of wrong than a redundant refresh, so every mutation
   * here reports.
   */
  const commit = useCallback(() => {
    reload();
    onChanged();
  }, [reload, onChanged]);

  /** Runs a pane-level action, reporting whatever sentence it returns. */
  const run = useCallback(
    async (key: string, task: () => Promise<string>) => {
      setBusy(key);
      try {
        toast(await task());
        commit();
      } catch (cause) {
        toast(errorMessage(cause), "error");
      } finally {
        setBusy(null);
      }
    },
    [toast, commit],
  );

  const groups = useMemo(() => groupByCategory(rules), [rules]);
  // Derived from the rules on screen rather than from `rules.categories()`.
  // The command returns exactly this - the distinct categories of the stored
  // rules plus 未分类 - so a second round trip would only add a way for the
  // combobox to disagree with the list the user is looking at.
  const categories = useMemo(() => categoriesOf(rules), [rules]);
  const disabledCount = rules.filter((rule) => !rule.enabled).length;

  async function saveRule(rule: Rule) {
    await rulesApi.save(rule);
    setEditing(null);
    commit();
    toast(rule.id === null ? "规则已添加" : "规则已保存");
  }

  async function toggleRule(rule: Rule) {
    setBusy(`toggle-${rule.id}`);
    try {
      await rulesApi.save({ ...rule, enabled: !rule.enabled });
      commit();
    } catch (cause) {
      toast(errorMessage(cause), "error");
    } finally {
      setBusy(null);
    }
  }

  async function removeRule(rule: Rule) {
    const id = rule.id;
    if (id === null) return;
    setConfirming(null);
    await run(`delete-${id}`, async () => {
      await rulesApi.remove(id);
      return `已删除「${rule.name}」`;
    });
  }

  /**
   * The dialog half of export/import.
   *
   * A null path means the user closed the dialog, which is not a failure and
   * gets no toast - the only thing worse than an app that swallows errors is
   * one that reports non-events. Anything the dialog itself throws is real and
   * is reported; anything the command throws is reported verbatim by `run`,
   * because those strings are already Chinese sentences written for the user.
   */
  async function exportToFile() {
    try {
      const path = await save({
        title: "导出分类规则",
        defaultPath: DEFAULT_RULES_FILENAME,
        filters: RULES_FILE_FILTER,
      });
      if (path === null) return;
      await run("export", async () => {
        const count = await rulesApi.exportToFile(path);
        return `已导出 ${count} 条规则`;
      });
    } catch (cause) {
      toast(errorMessage(cause), "error");
    }
  }

  async function importFromFile() {
    try {
      const path = await open({
        title: "导入分类规则",
        multiple: false,
        directory: false,
        filters: RULES_FILE_FILTER,
      });
      if (path === null) return;
      await run("import", async () => {
        const count = await rulesApi.importFromFile(path);
        return `已导入 ${count} 条规则`;
      });
    } catch (cause) {
      toast(errorMessage(cause), "error");
    }
  }

  return (
    <div className="rules-pane">
      <div className="toolbar">
        <Button intent="primary" onClick={() => setEditing(blankRule())}>
          <PlusIcon />
          新建规则
        </Button>
        <span className="rules-count">
          共 {rules.length} 条 · {groups.length} 个类别
          {disabledCount > 0 && ` · ${disabledCount} 条已停用`}
        </span>
        <span className="toolbar-spacer" />
        <Button
          onClick={() =>
            void run("reclassify", async () => {
              const changed = await invoicesApi.reclassifyAll();
              return changed === 0
                ? "重新分类完成，没有发票改变类别"
                : `重新分类完成，${changed} 张发票的类别有变化`;
            })
          }
          disabled={busy === "reclassify"}
        >
          <RefreshIcon />
          {busy === "reclassify" ? "重新分类中…" : "重新分类全部发票"}
        </Button>
      </div>

      <div className="main-scroll">
        <div className="rules-body">
          {/* The two facts that make the list readable. Both belong on screen
              rather than in a tooltip: the AND/OR split is the single thing
              people get wrong about rule editors, and "editing a rule does
              nothing to invoices you already imported" is the surprise that
              otherwise gets reported as a bug. */}
          <p className="rules-intro">
            一条规则里的多个条件必须<strong>同时满足</strong>
            ；同一个类别下的多条规则
            <strong>满足任意一条</strong>即可。优先级高的规则说了算。
          </p>
          <p className="rules-intro rules-intro-muted">
            改完规则要点「重新分类全部发票」才会影响已经导入的发票；你手动指定过类别的发票会保持原样，不会被规则改掉。
          </p>

          {error && <p className="rules-error">{error}</p>}

          {!loading && rules.length === 0 && (
            <div className="empty">
              <div className="empty-title">还没有任何分类规则</div>
              <p className="empty-hint">
                没有规则时所有发票都会落在「未分类」。可以先恢复内置规则，它覆盖了住宿、餐饮、交通这些最常见的开销。
              </p>
              <Button
                intent="primary"
                onClick={() => void run("restore", restoreDefaults)}
                disabled={busy === "restore"}
              >
                恢复内置规则
              </Button>
            </div>
          )}

          {groups.map((group) => (
            <section className="group rule-group" key={group.category}>
              <h3 className="group-title rule-group-title">
                <span>{group.category}</span>
                <span className="rule-group-count">
                  {group.rules.length} 条
                </span>
              </h3>
              <div className="group-body">
                {group.rules.map((rule) => (
                  <div
                    className={`rule-row ${rule.enabled ? "" : "rule-row-off"}`}
                    key={rule.id ?? rule.name}
                  >
                    <Toggle
                      checked={rule.enabled}
                      disabled={busy === `toggle-${rule.id}`}
                      onChange={() => void toggleRule(rule)}
                    />

                    <div className="rule-main">
                      <div
                        className="rule-conditions"
                        title={describeRule(rule)}
                      >
                        {rule.conditions.length === 0 ? (
                          <span className="rule-broken">
                            没有条件，这条规则不会匹配任何发票
                          </span>
                        ) : (
                          rule.conditions.map((condition, index) => (
                            <span className="rule-cond" key={index}>
                              {index > 0 && (
                                <span className="rule-joiner">
                                  {CONDITION_JOINER}
                                </span>
                              )}
                              <span className="rule-field">
                                {MATCH_FIELD_LABEL[condition.field]}
                              </span>
                              <span className="rule-verb">{CONTAINS}</span>
                              {condition.keywords.map((keyword, position) => (
                                <span key={keyword}>
                                  {position > 0 && (
                                    <span className="rule-or">
                                      {KEYWORD_JOINER}
                                    </span>
                                  )}
                                  <span className="rule-keyword">
                                    {keyword}
                                  </span>
                                </span>
                              ))}
                            </span>
                          ))
                        )}
                      </div>
                      {/* Only when the user renamed it. For every built-in the
                          name IS the条件 prose above, and repeating it on a
                          second line would double the height of a list whose
                          whole job is to be scannable. */}
                      {!isAutoName(rule) && (
                        <div className="rule-name">记为「{rule.name}」</div>
                      )}
                    </div>

                    <Badge
                      tone={rule.priority > 200 ? "warn" : "neutral"}
                      title={bandTitle(rule.priority)}
                    >
                      <span className="tnum">{rule.priority}</span>
                    </Badge>

                    {rule.id !== null && confirming === rule.id ? (
                      <div className="rule-actions">
                        <Button
                          intent="danger"
                          onClick={() => void removeRule(rule)}
                        >
                          确认删除
                        </Button>
                        <Button onClick={() => setConfirming(null)}>
                          取消
                        </Button>
                      </div>
                    ) : (
                      <div className="rule-actions">
                        <Button onClick={() => setEditing(rule)}>编辑</Button>
                        <Button
                          icon
                          intent="danger"
                          title="删除规则"
                          onClick={() => setConfirming(rule.id)}
                        >
                          <TrashIcon />
                        </Button>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </section>
          ))}

          <Group
            title="规则集维护"
            hint="规则是几个月攒出来的东西，所以能导出成文件带走，也能从别人那里导入。"
          >
            <Row
              label="恢复内置规则"
              hint="只补回你现在没有的内置规则，不会覆盖、也不会删掉你改过的任何一条。"
            >
              <Button
                onClick={() => void run("restore", restoreDefaults)}
                disabled={busy === "restore"}
              >
                {busy === "restore" ? "恢复中…" : "恢复内置规则"}
              </Button>
            </Row>

            <Row
              label="导出规则"
              hint="存成一个 .json 文件，可以拿去备份，也可以直接发给同事。"
            >
              <Button
                disabled={busy === "export"}
                onClick={() => void exportToFile()}
              >
                {busy === "export" ? "导出中…" : "导出规则"}
              </Button>
            </Row>

            <Row
              label="导入规则"
              hint="读入一份导出的 .json。导入的规则一律新增，不会覆盖你现有的任何一条。"
            >
              <Button
                disabled={busy === "import"}
                onClick={() => void importFromFile()}
              >
                {busy === "import" ? "导入中…" : "导入规则"}
              </Button>
            </Row>
          </Group>
        </div>
      </div>

      {editing && (
        <RuleEditor
          rule={editing}
          categories={categories}
          onSave={saveRule}
          onCancel={() => setEditing(null)}
        />
      )}
    </div>
  );
}

async function restoreDefaults(): Promise<string> {
  const added = await rulesApi.restoreDefaults();
  return added === 0
    ? "内置规则都在，没有需要补回的"
    : `已补回 ${added} 条内置规则`;
}

/** The tooltip on a priority badge: which band this number sits in. */
function bandTitle(priority: number): string {
  const band = PRIORITY_BANDS.find((entry) => entry.value === priority);
  return band
    ? `优先级 ${priority}（${band.label}）：${band.hint}`
    : `优先级 ${priority}`;
}
