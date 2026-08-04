/**
 * The review drawer - where the app either keeps or breaks its promise.
 *
 * The promise is: **it never silently claims a number it is not sure of.**
 * Everything here follows from that. Every field shows where its value came
 * from (`SOURCE_LABEL`), anything below `REVIEW_THRESHOLD` wears the amber
 * `.input-invalid` treatment, the 明细 table shows the `*税收分类简称*` the
 * classifier keyed on, and the 来源 section can put the original file on
 * screen in one click so the user can compare. A number the user retypes
 * becomes a `manualField`, which outranks every machine source in
 * `Field::merge_from` - that, and nothing else, is what stops a re-scan from
 * quietly undoing a correction.
 *
 * It is a drawer over the table rather than a modal because reviewing is a
 * sequence: the list stays visible and clickable, and ↑/↓ walk the rows
 * without touching the mouse, so a month of invoices can be checked in one
 * pass from the keyboard.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  TRUST_LABEL,
  fieldsNeedingReview,
  formatMoney,
  isError,
  issueMessage,
  trustOf,
} from "../format";
import {
  errorMessage,
  invoices as invoicesApi,
  rules as rulesApi,
} from "../ipc";
import {
  KIND_LABEL,
  MATCH_FIELD_LABEL,
  SOURCE_LABEL,
  emptyField,
  manualField,
  needsReview,
  type Cents,
  type Field,
  type Invoice,
  type InvoiceItem,
  type Suggestion,
} from "../types";
import {
  ChevronIcon,
  CloseIcon,
  ExternalIcon,
  RefreshIcon,
  SparkIcon,
  TrashIcon,
} from "../ui/icons";
import {
  Badge,
  Button,
  Group,
  Modal,
  Row,
  Select,
  TextInput,
  useAsync,
  useToast,
} from "../ui/primitives";
import { DateInput } from "./DateInput";
import { parseYuan, yuanInput } from "./money";
import { UNCATEGORISED } from "./rows";

export interface InvoiceDetailProps {
  id: number;
  /**
   * The row's stored review state. Passed in because `Invoice` does not carry
   * it - it lives on the row, not in the payload - and `invoices.save` needs
   * it: a plain 保存 must not silently un-review a row that was already
   * checked.
   */
  reviewed: boolean;
  categories: string[];
  /** 0-based position in the list behind the drawer; -1 when not in it. */
  index: number;
  count: number;
  hasPrev: boolean;
  hasNext: boolean;
  onNavigate: (delta: number) => void;
  onClose: () => void;
  /** Anything that changes counts or totals, so the sidebar stays honest. */
  onChanged: () => void;
  /**
   * Reports unsaved edits upwards.
   *
   * The drawer guards its own exits (Escape, ↑/↓, the close button), but it
   * cannot guard a click on a different row in the table behind it - so the
   * pane needs to know, and refuses to swap the drawer's invoice out from
   * under an edit in progress.
   */
  onDirtyChange: (dirty: boolean) => void;
}

type MoneyKey = "amountExclTax" | "tax" | "total";
const MONEY_KEYS: MoneyKey[] = ["amountExclTax", "tax", "total"];
type MoneyText = Record<MoneyKey, string>;
const EMPTY_MONEY: MoneyText = { amountExclTax: "", tax: "", total: "" };

/** What the user asked to do that unsaved edits are standing in the way of. */
type Leave = { kind: "close" } | { kind: "nav"; delta: number };

export function InvoiceDetail({
  id,
  reviewed,
  categories,
  index,
  count,
  hasPrev,
  hasNext,
  onNavigate,
  onClose,
  onChanged,
  onDirtyChange,
}: InvoiceDetailProps) {
  const toast = useToast();

  const query = useAsync<Invoice | null>(() => invoicesApi.get(id), [id], null);

  const [draft, setDraft] = useState<Invoice | null>(null);
  const [moneyText, setMoneyText] = useState<MoneyText>(EMPTY_MONEY);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [suggestion, setSuggestion] = useState<Suggestion | null>(null);
  const [suggesting, setSuggesting] = useState(false);
  const [pendingLeave, setPendingLeave] = useState<Leave | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const resetFrom = useCallback((invoice: Invoice | null) => {
    setDraft(invoice);
    setMoneyText(
      invoice
        ? {
            amountExclTax: yuanInput(invoice.amountExclTax.value),
            tax: yuanInput(invoice.tax.value),
            total: yuanInput(invoice.total.value),
          }
        : EMPTY_MONEY,
    );
    setDirty(false);
    setSuggestion(null);
  }, []);

  // A reload landing mid-edit must not throw the edit away. Navigation is
  // already blocked while dirty, so in practice this only guards the reloads
  // this component triggers itself (after saving a rule, say).
  const dirtyRef = useRef(false);
  // Mirrored into a ref in its own effect, declared BEFORE the reset below so
  // that within a single commit the flag is already current when the reset
  // effect reads it.
  useEffect(() => {
    dirtyRef.current = dirty;
  }, [dirty]);
  useEffect(() => {
    if (dirtyRef.current) return;
    resetFrom(query.data);
  }, [query.data, resetFrom]);

  useEffect(() => {
    if (query.error) toast(query.error, "error");
  }, [query.error, toast]);

  // The cleanup clears the flag on unmount, so closing the drawer can never
  // leave the pane believing an edit is still open somewhere.
  useEffect(() => {
    onDirtyChange(dirty);
    return () => onDirtyChange(false);
  }, [dirty, onDirtyChange]);

  const edit = useCallback((change: (invoice: Invoice) => Invoice) => {
    setDraft((current) => (current ? change(current) : current));
    setDirty(true);
  }, []);

  // ---------------------------------------------------------------------
  // Leaving, and the edits that stand in the way
  // ---------------------------------------------------------------------

  const performLeave = useCallback(
    (intent: Leave) => {
      if (intent.kind === "close") onClose();
      else onNavigate(intent.delta);
    },
    [onClose, onNavigate],
  );

  const requestLeave = useCallback(
    (intent: Leave) => {
      if (dirty) {
        setPendingLeave(intent);
        return;
      }
      performLeave(intent);
    },
    [dirty, performLeave],
  );

  /**
   * Escape closes; ↑/↓ walk the list.
   *
   * Two guards make this liveable. Arrows are ignored while focus is in a
   * field, or correcting a 发票号码 would jump to another invoice halfway
   * through typing it. And the whole handler stands down while a confirm
   * dialog is open, so Escape cancels the dialog (the Modal's own handler)
   * instead of racing it.
   */
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (pendingLeave || confirmDelete) return;
      // Any other dialog on screen - the pane's bulk-delete confirmation, say
      // - owns the keyboard while it is up. Checking the DOM rather than
      // threading a prop through: whoever opened it is not necessarily
      // someone this component has heard of.
      if (document.querySelector(".modal-backdrop")) return;
      if (event.key === "Escape") {
        requestLeave({ kind: "close" });
        return;
      }
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable);
      if (typing) return;
      if (event.key === "ArrowDown" && hasNext) {
        event.preventDefault();
        requestLeave({ kind: "nav", delta: 1 });
      } else if (event.key === "ArrowUp" && hasPrev) {
        event.preventDefault();
        requestLeave({ kind: "nav", delta: -1 });
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [requestLeave, pendingLeave, confirmDelete, hasNext, hasPrev]);

  // ---------------------------------------------------------------------
  // Money: 元 in, 分 stored
  // ---------------------------------------------------------------------

  /**
   * Keeps the typed text and the stored `Cents` as separate state.
   *
   * They have to be separate because "10." and "" are legitimate things to
   * have in a field mid-keystroke and neither is a number. The rule is: blank
   * clears the field to "not present"; anything that parses is stored as
   * `manualField(cents)`; anything else leaves the stored value ALONE and
   * marks the input invalid. Nothing here can turn a typo into ¥0.00.
   */
  const onMoneyChange = useCallback((key: MoneyKey, text: string) => {
    setMoneyText((current) => ({ ...current, [key]: text }));
    setDirty(true);
    const trimmed = text.trim();
    const field: Field<Cents> | null =
      trimmed === "" ? emptyField<Cents>() : wrapParsed(parseYuan(trimmed));
    if (!field) return;
    setDraft((current) => (current ? withMoney(current, key, field) : current));
  }, []);

  const moneyInvalid = useCallback(
    (key: MoneyKey) => {
      const text = moneyText[key].trim();
      return text !== "" && parseYuan(text) === null;
    },
    [moneyText],
  );

  const anyMoneyInvalid = MONEY_KEYS.some(moneyInvalid);

  // ---------------------------------------------------------------------
  // Commands
  // ---------------------------------------------------------------------

  const save = useCallback(
    async (reviewedNext: boolean) => {
      if (!draft || anyMoneyInvalid) return;
      setBusy(true);
      try {
        await invoicesApi.save(id, draft, reviewedNext);
        setDirty(false);
        // The Rust side re-runs validation on save, so the issue list on
        // screen is stale the moment it returns - reload rather than leave a
        // fixed 价税合计 still wearing its "对不上" warning.
        query.reload();
        onChanged();
        toast(reviewedNext ? "已保存，并标记为已复核" : "已保存");
      } catch (error) {
        toast(errorMessage(error), "error");
      } finally {
        setBusy(false);
      }
    },
    [draft, anyMoneyInvalid, id, query, onChanged, toast],
  );

  const remove = useCallback(async () => {
    setBusy(true);
    try {
      await invoicesApi.remove(id);
      setConfirmDelete(false);
      onChanged();
      onClose();
    } catch (error) {
      toast(errorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  }, [id, onChanged, onClose, toast]);

  const rescan = useCallback(async () => {
    setBusy(true);
    try {
      const fresh = await invoicesApi.rescan(id);
      resetFrom(fresh);
      onChanged();
      toast("已重新识别，人工录入的字段保持不变");
    } catch (error) {
      toast(errorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  }, [id, resetFrom, onChanged, toast]);

  const openSource = useCallback(async () => {
    if (!draft) return;
    try {
      await invoicesApi.openSource(draft.sourcePath);
    } catch (error) {
      toast(errorMessage(error), "error");
    }
  }, [draft, toast]);

  /**
   * Sets the category on the stored row immediately, without waiting for 保存.
   *
   * It is patched into the draft as well as sent to the backend: `save` writes
   * the whole invoice, so a draft still carrying the old category would undo
   * this the next time the user pressed 保存.
   */
  const applyCategory = useCallback(
    async (category: string) => {
      setBusy(true);
      try {
        await invoicesApi.setCategory(id, category);
        // `set_invoice_category` clears the rule note, because the reason is
        // now "the user said so" and crediting a rule would be a lie.
        setDraft((current) =>
          current
            ? { ...current, category: category || null, categoryRule: null }
            : current,
        );
        onChanged();
      } catch (error) {
        toast(errorMessage(error), "error");
      } finally {
        setBusy(false);
      }
    },
    [id, onChanged, toast],
  );

  const askSuggestion = useCallback(async () => {
    setSuggesting(true);
    try {
      setSuggestion(await invoicesApi.suggestCategory(id));
    } catch (error) {
      // The backend's refusals are already user-facing sentences ("AI 分类
      // 建议未开启…"), so they go through verbatim.
      toast(errorMessage(error), "error");
    } finally {
      setSuggesting(false);
    }
  }, [id, toast]);

  /**
   * Keeps the rule, not just the answer.
   *
   * The suggestion itself fixes one row and costs one API call; the rule
   * behind it fixes every invoice from the same 税收分类 or vendor - the ones
   * already in the ledger and every future import - and costs nothing ever
   * again. That asymmetry is why the two actions are separate buttons rather
   * than an accept with a checkbox: saving the rule is the valuable half, and
   * it should look like the bigger decision, because it is one.
   */
  const saveAsRule = useCallback(
    async (accepted: Suggestion) => {
      setBusy(true);
      try {
        await rulesApi.save(accepted.proposedRule);
        const changed = await invoicesApi.reclassifyAll();
        setSuggestion(null);
        query.reload();
        onChanged();
        toast(
          `已保存规则「${accepted.proposedRule.name}」，重新归类 ${changed} 张`,
        );
      } catch (error) {
        toast(errorMessage(error), "error");
      } finally {
        setBusy(false);
      }
    },
    [query, onChanged, toast],
  );

  // ---------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------

  const pending = draft ? fieldsNeedingReview(draft) : [];
  const categoryOptions = useMemo(() => {
    const options = [
      { value: "", label: UNCATEGORISED },
      ...categories.map((name) => ({ value: name, label: name })),
    ];
    const current = draft?.category ?? "";
    if (current && !categories.includes(current)) {
      options.push({ value: current, label: current });
    }
    return options;
  }, [categories, draft?.category]);

  const computedTotal =
    draft && draft.amountExclTax.value !== null && draft.tax.value !== null
      ? draft.amountExclTax.value + draft.tax.value
      : null;

  return (
    <aside className="invoice-detail" aria-label="发票详情">
      <header className="detail-header">
        <div className="detail-title">
          <span className="detail-number tnum">
            {draft?.number.value ?? `#${id}`}
          </span>
          <span className="detail-kind">
            {draft?.kind ? KIND_LABEL[draft.kind] : "未识别票种"}
          </span>
        </div>
        <div className="detail-header-right">
          {pending.length > 0 && (
            <Badge tone="warn" title={pending.join("、")}>
              待复核 {pending.length} 项
            </Badge>
          )}
          {/* Neutral, not green: "this one is fine" is the normal case and
              does not deserve a colour of its own. */}
          {reviewed && pending.length === 0 && (
            <Badge tone="neutral">已复核</Badge>
          )}
          <span className="detail-position tnum">
            {index >= 0 ? `${index + 1}/${count}` : ""}
          </span>
          <button
            className="detail-nav"
            title="上一张（↑）"
            disabled={!hasPrev}
            onClick={() => requestLeave({ kind: "nav", delta: -1 })}
          >
            <ChevronIcon className="detail-nav-up" />
          </button>
          <button
            className="detail-nav"
            title="下一张（↓）"
            disabled={!hasNext}
            onClick={() => requestLeave({ kind: "nav", delta: 1 })}
          >
            <ChevronIcon className="detail-nav-down" />
          </button>
          <button
            className="detail-close"
            title="关闭（Esc）"
            onClick={() => requestLeave({ kind: "close" })}
          >
            <CloseIcon />
          </button>
        </div>
      </header>

      <div className="detail-body">
        {!draft && (
          <div className="detail-loading">
            {query.loading ? "读取中…" : "发票不存在"}
          </div>
        )}

        {draft && (
          <>
            {draft.issues.length > 0 && (
              <ul className="detail-issues">
                {draft.issues.map((issue, position) => (
                  <li
                    key={position}
                    className={`detail-issue ${isError(issue) ? "detail-issue-error" : ""}`}
                  >
                    {issueMessage(issue)}
                  </li>
                ))}
              </ul>
            )}

            <Group
              title="票面信息"
              hint="标黄的字段识别置信度不足，请对照原票核对。任何修改都会记为「人工录入」，重新识别不会覆盖它。"
            >
              <TextField
                label="发票号码"
                field={draft.number}
                mono
                onChange={(field) =>
                  edit((invoice) => ({ ...invoice, number: field }))
                }
              />
              <TextField
                label="发票代码"
                field={draft.code}
                mono
                placeholder="数电票没有发票代码"
                onChange={(field) =>
                  edit((invoice) => ({ ...invoice, code: field }))
                }
              />
              <Row label="开票日期" hint={sourceHint(draft.issuedOn)}>
                <DateInput
                  value={draft.issuedOn.value ?? ""}
                  invalid={needsReview(draft.issuedOn)}
                  onChange={(value) =>
                    edit((invoice) => ({
                      ...invoice,
                      issuedOn:
                        value === ""
                          ? emptyField<string>()
                          : manualField(value),
                    }))
                  }
                />
              </Row>
              <TextField
                label="备注"
                field={draft.remark}
                stacked
                onChange={(field) =>
                  edit((invoice) => ({ ...invoice, remark: field }))
                }
              />
            </Group>

            <Group title="交易双方">
              <TextField
                label="购买方名称"
                field={draft.buyerName}
                stacked
                onChange={(field) =>
                  edit((invoice) => ({ ...invoice, buyerName: field }))
                }
              />
              <TextField
                label="购买方税号"
                field={draft.buyerTaxId}
                mono
                stacked
                onChange={(field) =>
                  edit((invoice) => ({ ...invoice, buyerTaxId: field }))
                }
              />
              <TextField
                label="销售方名称"
                field={draft.sellerName}
                stacked
                onChange={(field) =>
                  edit((invoice) => ({ ...invoice, sellerName: field }))
                }
              />
              <TextField
                label="销售方税号"
                field={draft.sellerTaxId}
                mono
                stacked
                onChange={(field) =>
                  edit((invoice) => ({ ...invoice, sellerTaxId: field }))
                }
              />
            </Group>

            <Group
              title="金额"
              hint={
                computedTotal === null
                  ? "按元填写，例如 1060.00。填不出的字段留空，不要填 0。"
                  : `金额 + 税额 = ${formatMoney(computedTotal)}，应与价税合计一致。`
              }
            >
              <MoneyField
                label="金额(不含税)"
                field={draft.amountExclTax}
                text={moneyText.amountExclTax}
                invalid={moneyInvalid("amountExclTax")}
                onChange={(text) => onMoneyChange("amountExclTax", text)}
              />
              <MoneyField
                label="税额"
                field={draft.tax}
                text={moneyText.tax}
                invalid={moneyInvalid("tax")}
                onChange={(text) => onMoneyChange("tax", text)}
              />
              <MoneyField
                label="价税合计"
                field={draft.total}
                text={moneyText.total}
                invalid={moneyInvalid("total")}
                onChange={(text) => onMoneyChange("total", text)}
              />
            </Group>

            <Group title="分类">
              <Row
                label="费用类别"
                hint={
                  draft.categoryRule
                    ? `由规则「${draft.categoryRule}」判定`
                    : draft.category
                      ? "手动设置"
                      : "还没有规则认领这张票"
                }
              >
                <Select
                  value={draft.category ?? ""}
                  options={categoryOptions}
                  disabled={busy}
                  onChange={(value) => void applyCategory(value)}
                />
              </Row>

              <Row
                label="AI 建议分类"
                hint="把票面信息交给模型，请它给出类别和一条可复用的规则。需要在设置里开启。"
              >
                <Button
                  onClick={() => void askSuggestion()}
                  disabled={suggesting || busy}
                >
                  <SparkIcon />
                  {suggesting ? "询问中…" : "问一次"}
                </Button>
              </Row>

              {suggestion && (
                <div className="suggestion">
                  <div className="suggestion-head">
                    <Badge tone="accent">{suggestion.category}</Badge>
                    <button
                      className="detail-close"
                      title="忽略建议"
                      onClick={() => setSuggestion(null)}
                    >
                      <CloseIcon />
                    </button>
                  </div>
                  <p className="suggestion-reason">{suggestion.reason}</p>
                  <div className="suggestion-rule">
                    <div className="suggestion-rule-name">
                      规则「{suggestion.proposedRule.name}」· 优先级{" "}
                      {suggestion.proposedRule.priority}
                    </div>
                    {suggestion.proposedRule.conditions.map(
                      (condition, position) => (
                        <div key={position} className="suggestion-condition">
                          {MATCH_FIELD_LABEL[condition.field]} 包含 “
                          {condition.keywords.join("”、“")}”
                        </div>
                      ),
                    )}
                  </div>
                  <div className="suggestion-actions">
                    <Button
                      onClick={() => void applyCategory(suggestion.category)}
                      disabled={busy}
                    >
                      只用于这张
                    </Button>
                    <Button
                      intent="primary"
                      onClick={() => void saveAsRule(suggestion)}
                      disabled={busy}
                    >
                      保存为规则
                    </Button>
                  </div>
                  <p className="suggestion-note">
                    保存为规则后，以后同类发票会自动归类，已导入的也会重新归类一遍。
                  </p>
                </div>
              )}
            </Group>

            {draft.items.length > 0 && (
              <Group
                title="明细"
                hint="名称里 *星号* 之间的是税收分类简称，税局统一口径，也是分类规则最可靠的匹配依据。"
              >
                <table className="detail-items">
                  <thead>
                    <tr>
                      <th>名称</th>
                      <th className="cell-num">数量</th>
                      <th className="cell-num">单价</th>
                      <th className="cell-num">金额</th>
                      <th className="cell-num">税率</th>
                      <th className="cell-num">税额</th>
                    </tr>
                  </thead>
                  <tbody>
                    {draft.items.map((item, position) => (
                      <ItemRow key={position} item={item} />
                    ))}
                  </tbody>
                </table>
              </Group>
            )}

            <Group
              title="来源"
              hint="每个字段旁边写着它是从哪一层读出来的，这就是这张票的识别过程。"
            >
              <Row
                label="原始文件"
                hint={<span className="detail-path">{draft.sourcePath}</span>}
                stacked
              >
                <Button onClick={() => void openSource()}>
                  <ExternalIcon />
                  打开原文件
                </Button>
              </Row>
              <Row
                label="重新识别"
                hint="重新读一遍原文件，只补上空缺的字段；人工录入的内容不会被覆盖。"
              >
                <Button onClick={() => void rescan()} disabled={busy}>
                  <RefreshIcon />
                  重新识别
                </Button>
              </Row>
            </Group>
          </>
        )}
      </div>

      <footer className="detail-footer">
        <Button
          intent="danger"
          onClick={() => setConfirmDelete(true)}
          disabled={busy}
        >
          <TrashIcon />
          删除
        </Button>
        <span className="detail-footer-spacer" />
        {dirty && <span className="detail-dirty">未保存</span>}
        <Button
          onClick={() => void save(reviewed)}
          disabled={!draft || busy || anyMoneyInvalid}
        >
          保存
        </Button>
        <Button
          intent="primary"
          onClick={() => void save(true)}
          disabled={!draft || busy || anyMoneyInvalid}
        >
          {reviewed ? "保存并保持已复核" : "标记已复核"}
        </Button>
      </footer>

      {pendingLeave && (
        <Modal
          title="还有未保存的修改"
          onClose={() => setPendingLeave(null)}
          footer={
            <>
              <Button onClick={() => setPendingLeave(null)}>继续编辑</Button>
              <Button
                intent="danger"
                onClick={() => {
                  const intent = pendingLeave;
                  setPendingLeave(null);
                  setDirty(false);
                  performLeave(intent);
                }}
              >
                放弃修改
              </Button>
            </>
          }
        >
          这张发票上的修改还没有保存，离开会丢掉它们。
        </Modal>
      )}

      {confirmDelete && (
        <Modal
          title="删除这张发票"
          onClose={() => setConfirmDelete(false)}
          footer={
            <>
              <Button onClick={() => setConfirmDelete(false)}>取消</Button>
              <Button
                intent="danger"
                onClick={() => void remove()}
                disabled={busy}
              >
                删除
              </Button>
            </>
          }
        >
          只从 ZhiShui 的账本里删除记录，原始文件不会被动。
        </Modal>
      )}
    </aside>
  );
}

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

/** `null` in, `null` out - an unparseable amount must not touch the draft. */
function wrapParsed(cents: Cents | null): Field<Cents> | null {
  return cents === null ? null : manualField(cents);
}

/** Typed per key rather than with a computed key, so nothing widens to `any`. */
function withMoney(
  invoice: Invoice,
  key: MoneyKey,
  field: Field<Cents>,
): Invoice {
  switch (key) {
    case "amountExclTax":
      return { ...invoice, amountExclTax: field };
    case "tax":
      return { ...invoice, tax: field };
    case "total":
      return { ...invoice, total: field };
  }
}

/**
 * The provenance line under a field's label.
 *
 * Shown for every field, not only the doubtful ones: "PDF 文本层" beside a
 * number that looks right is what makes "AI 识别" beside the next one mean
 * something.
 */
function sourceHint<T>(field: Field<T>): ReactNode {
  const trust = trustOf(field);
  return (
    <span
      className={
        trust === "ok" ? "field-source" : "field-source field-source-review"
      }
    >
      {SOURCE_LABEL[field.source]}
      {trust === "ok" ? "" : ` · ${TRUST_LABEL[trust]}`}
    </span>
  );
}

function TextField({
  label,
  field,
  onChange,
  mono,
  stacked,
  placeholder,
}: {
  label: string;
  field: Field<string>;
  onChange: (field: Field<string>) => void;
  mono?: boolean;
  stacked?: boolean;
  placeholder?: string;
}) {
  return (
    <Row label={label} hint={sourceHint(field)} stacked={stacked}>
      <TextInput
        value={field.value ?? ""}
        mono={mono}
        placeholder={placeholder}
        invalid={needsReview(field)}
        // Only the emptiness test trims, so a space typed inside a company
        // name survives; a field cleared to nothing goes back to "not
        // present" rather than being stored as an empty manual value, which
        // would tell a later re-scan there is nothing left to find.
        onChange={(text) =>
          onChange(
            text.trim() === "" ? emptyField<string>() : manualField(text),
          )
        }
      />
    </Row>
  );
}

function MoneyField({
  label,
  field,
  text,
  invalid,
  onChange,
}: {
  label: string;
  field: Field<Cents>;
  text: string;
  invalid: boolean;
  onChange: (text: string) => void;
}) {
  return (
    <Row
      label={label}
      hint={
        invalid ? (
          <span className="field-source-review">看不懂这个金额</span>
        ) : (
          sourceHint(field)
        )
      }
    >
      <div className="money-input">
        <span className="money-unit">¥</span>
        <TextInput
          value={text}
          mono
          placeholder="0.00"
          invalid={invalid || needsReview(field)}
          onChange={onChange}
        />
      </div>
    </Row>
  );
}

function ItemRow({ item }: { item: InvoiceItem }) {
  return (
    <tr>
      <td className="item-name">
        <ItemName name={item.name} />
      </td>
      <td className="cell-num tnum">{item.quantity ?? ""}</td>
      <td className="cell-num tnum">
        {item.unitPrice === null ? "" : formatMoney(item.unitPrice)}
      </td>
      <td className="cell-num tnum">
        {item.amount === null ? "" : formatMoney(item.amount)}
      </td>
      <td className="cell-num">{item.taxRate ?? ""}</td>
      <td className="cell-num tnum">
        {item.tax === null ? "" : formatMoney(item.tax)}
      </td>
    </tr>
  );
}

/**
 * Draws `*住宿服务*住宿费` with the starred prefix picked out.
 *
 * Worth the highlight because that prefix is the tax authority's own
 * classification and the strongest signal the rule engine has - a user who
 * learns to read it can write a rule that categorises a whole trade correctly
 * on the first try. Mirrors `InvoiceItem::tax_category`.
 */
function ItemName({ name }: { name: string }) {
  const match = /^\*([^*]+)\*(.*)$/.exec(name);
  if (!match) return <span>{name}</span>;
  return (
    <>
      <span className="tax-category">*{match[1]}*</span>
      {match[2]}
    </>
  );
}
