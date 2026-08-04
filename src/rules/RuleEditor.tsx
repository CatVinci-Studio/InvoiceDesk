/**
 * The one rule form.
 *
 * Takes a rule and gives one back - it does no IPC of its own, which keeps the
 * whole thing testable without a Tauri host and keeps every write to the rule
 * set funnelled through `RulesPane`.
 *
 * The form is arranged around the one question a user actually has when they
 * open it: *what will this rule claim, and what happens when something else
 * claims it too*. Hence the running prose preview at the bottom, the priority
 * bands beside the number, and the AND/OR line above the conditions.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import {
  MATCH_FIELD_HINT,
  MATCH_FIELD_LABEL,
  type Condition,
  type MatchField,
  type Rule,
} from "../types";
import { CloseIcon, PlusIcon, TrashIcon } from "../ui/icons";
import { Button, Group, Modal, Row, Select, TextInput } from "../ui/primitives";
import {
  CONDITION_JOINER,
  PRIORITY_BANDS,
  defaultRuleName,
  describeRule,
  emptyCondition,
  isAutoName,
  priorityNote,
  validateDraft,
} from "./ruleText";
import "./rules.css";

const MATCH_FIELDS: MatchField[] = [
  "taxCategory",
  "itemName",
  "sellerName",
  "remark",
  "kind",
];

interface RuleEditorProps {
  /** The rule to edit, or a blank draft from `blankRule()`. */
  rule: Rule;
  /** Categories already in use, for the combobox. New ones may be typed. */
  categories: string[];
  /** Resolves once the rule is stored; the editor stays up if it rejects. */
  onSave: (rule: Rule) => Promise<void>;
  onCancel: () => void;
}

export function RuleEditor({
  rule,
  categories,
  onSave,
  onCancel,
}: RuleEditorProps) {
  const [category, setCategory] = useState(rule.category);
  // Empty means "use the generated name". An existing rule whose stored name
  // is exactly what we would have generated is treated as never having been
  // named, so editing its conditions updates the name with them instead of
  // leaving 「税收分类简称：住宿服务」 on a rule that now matches something
  // else entirely. A name the user actually typed is never touched.
  const [nameOverride, setNameOverride] = useState(() =>
    isAutoName(rule) ? "" : rule.name,
  );
  // Kept as text so the field can be empty mid-edit rather than snapping to 0.
  const [priorityText, setPriorityText] = useState(String(rule.priority));
  const [conditions, setConditions] = useState<Condition[]>(() =>
    rule.conditions.length > 0
      ? rule.conditions.map((condition) => ({
          ...condition,
          keywords: [...condition.keywords],
        }))
      : [emptyCondition()],
  );
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const autoName = defaultRuleName(conditions);
  const priorityIsInteger = /^-?\d+$/.test(priorityText.trim());
  const priority = priorityIsInteger
    ? Number.parseInt(priorityText.trim(), 10)
    : NaN;

  const draft: Rule = useMemo(
    () => ({
      ...rule,
      category: category.trim(),
      name: nameOverride.trim() || autoName,
      priority,
      conditions,
    }),
    [rule, category, nameOverride, autoName, priority, conditions],
  );

  const problem = priorityIsInteger
    ? validateDraft(draft)
    : "优先级必须填一个整数";
  const note = priorityIsInteger ? priorityNote(priority) : null;

  const updateCondition = (index: number, next: Condition) =>
    setConditions((current) => current.map((c, i) => (i === index ? next : c)));

  async function save() {
    if (problem) return;
    setSaving(true);
    setFailure(null);
    try {
      await onSave(draft);
    } catch (error) {
      // Whatever the backend rejected with is already a sentence for the user;
      // it is shown in place rather than as a toast because the form is still
      // open and the thing to fix is in it.
      setFailure(typeof error === "string" ? error : String(error));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal
      wide
      title={rule.id === null ? "新建分类规则" : "编辑分类规则"}
      onClose={onCancel}
      footer={
        <>
          {(failure ?? problem) && (
            <span className="rule-problem">{failure ?? problem}</span>
          )}
          <Button onClick={onCancel} disabled={saving}>
            取消
          </Button>
          <Button
            intent="primary"
            onClick={() => void save()}
            disabled={!!problem || saving}
          >
            {saving ? "保存中…" : "保存"}
          </Button>
        </>
      }
    >
      <Group title="归类">
        <Row
          label="费用类别"
          hint="直接输入就能新建类别，不用先去别处定义；报销单按这个类别汇总。"
        >
          <CategoryCombobox
            value={category}
            onChange={setCategory}
            categories={categories}
          />
        </Row>

        <Row
          label="规则名称"
          hint="发票上「为什么是这个类别」显示的就是这个名字，所以写成一句理由。留空会按条件自动生成。"
        >
          <TextInput
            value={nameOverride}
            onChange={setNameOverride}
            placeholder={autoName || "填好条件后自动生成"}
          />
        </Row>

        <Row
          label="优先级"
          hint="数字大的说了算。同一张发票常常有好几条规则都命中，优先级决定「为什么是这个类别」写的是哪一条。"
          stacked
        >
          <div className="rule-priority-field">
            <div className="rule-priority-input">
              <TextInput value={priorityText} onChange={setPriorityText} mono />
              {note && (
                <span
                  className={`rule-band-note ${note.warn ? "rule-band-warn" : ""}`}
                >
                  {note.text}
                </span>
              )}
            </div>
            <div className="rule-bands">
              {PRIORITY_BANDS.map((band) => (
                <button
                  key={band.value}
                  type="button"
                  className={`rule-band ${
                    priority === band.value ? "rule-band-active" : ""
                  }`}
                  title={band.hint}
                  onClick={() => setPriorityText(String(band.value))}
                >
                  <span className="rule-band-value tnum">{band.value}</span>
                  {band.label}
                </button>
              ))}
            </div>
          </div>
        </Row>
      </Group>

      <Group
        title="匹配条件"
        hint="下面每一条都要满足（并且）。想表达「或者」，就为同一个类别再建一条规则。"
      >
        {conditions.map((condition, index) => (
          <div className="cond" key={index}>
            <div className="cond-head">
              <span className="cond-index">
                {index === 0 ? "当" : CONDITION_JOINER}
              </span>
              <Select<MatchField>
                value={condition.field}
                onChange={(field) =>
                  updateCondition(index, { ...condition, field })
                }
                options={MATCH_FIELDS.map((field) => ({
                  value: field,
                  label: MATCH_FIELD_LABEL[field],
                }))}
              />
              <span className="cond-verb">包含以下任意一个</span>
              <span className="cond-spacer" />
              <Button
                icon
                intent="danger"
                title="删除这个条件"
                onClick={() =>
                  setConditions((current) =>
                    current.filter((_, i) => i !== index),
                  )
                }
              >
                <TrashIcon />
              </Button>
            </div>
            <p className="cond-hint">{MATCH_FIELD_HINT[condition.field]}</p>
            <KeywordChips
              keywords={condition.keywords}
              onChange={(keywords) =>
                updateCondition(index, { ...condition, keywords })
              }
            />
          </div>
        ))}

        <div className="cond-add">
          <Button
            onClick={() =>
              setConditions((current) => [...current, emptyCondition()])
            }
          >
            <PlusIcon />
            添加条件
          </Button>
        </div>
      </Group>

      {/* The whole rule read back as one sentence. It is the same text the
          list shows, so what the user is about to save is what they will see
          tomorrow when they ask why an invoice landed where it did. */}
      <p className="rule-preview">
        <span className="rule-preview-label">这条规则的意思是</span>
        {conditions.some((c) => c.keywords.length > 0)
          ? `${describeRule(draft)} → 归入「${draft.category || "…"}」`
          : "…"}
      </p>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Category combobox
// ---------------------------------------------------------------------------

/**
 * Pick an existing category or type a new one.
 *
 * A plain `<select>` would have been less code, but categories are not a fixed
 * vocabulary - every company has a 费用类别 list of its own, and the app has no
 * business insisting on the seven it happens to ship. So the field is a text
 * input first and a list second, and the "此类别是新的" line makes the implicit
 * creation explicit: nothing anywhere else in the app creates a category, they
 * come into existence by being used here.
 */
function CategoryCombobox({
  value,
  onChange,
  categories,
}: {
  value: string;
  onChange: (value: string) => void;
  categories: string[];
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Deliberately not the shared `useCloseOnOutsideClick`: that hook keeps its
  // listeners attached whether or not anything is open, and its Escape handler
  // runs in the bubble phase - so pressing Escape with this menu open would
  // also reach the Modal's own document-level handler and close the entire
  // editor. Capturing at the document and stopping propagation there means
  // Escape peels off one layer at a time, which is what the key is for.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node))
        setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);

  const typed = value.trim();
  const matches = categories.filter(
    (category) => !typed || category.includes(typed),
  );
  const isNew = typed.length > 0 && !categories.includes(typed);

  return (
    <div className="combo" ref={ref}>
      <input
        className="input combo-input"
        value={value}
        placeholder="例如 差旅费"
        onChange={(event) => {
          onChange(event.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
      />
      {isNew && <span className="combo-new">将新建类别「{typed}」</span>}
      {open && matches.length > 0 && (
        <div className="combo-menu">
          {matches.map((category) => (
            <button
              key={category}
              type="button"
              className={`combo-item ${category === typed ? "combo-item-active" : ""}`}
              onClick={() => {
                onChange(category);
                setOpen(false);
              }}
            >
              {category}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Keyword chips
// ---------------------------------------------------------------------------

/** What a user pasting a list of keywords is likely to have between them. */
const SEPARATORS = /[,，、;；\n\t]/;

/**
 * Keywords as chips rather than a comma-separated string.
 *
 * The string version is faster to type and much worse to read back: 住宿服务,
 * 客房, 住宿 is one blob, and a stray full-width comma or a trailing space
 * silently becomes part of a keyword that then never matches anything. Chips
 * make each keyword a thing you can see and delete, and the trimming happens
 * at the moment of entry where it cannot surprise anyone.
 */
function KeywordChips({
  keywords,
  onChange,
}: {
  keywords: string[];
  onChange: (keywords: string[]) => void;
}) {
  const [draft, setDraft] = useState("");

  const commit = (raw: string) => {
    const added = raw
      .split(SEPARATORS)
      .map((part) => part.trim())
      .filter(Boolean);
    if (added.length === 0) {
      setDraft("");
      return;
    }
    const next = [...keywords];
    for (const keyword of added) {
      if (!next.includes(keyword)) next.push(keyword);
    }
    onChange(next);
    setDraft("");
  };

  return (
    <div className="chips">
      {keywords.map((keyword) => (
        <span className="chip" key={keyword}>
          {keyword}
          <button
            type="button"
            className="chip-remove"
            title="删除关键词"
            onClick={() => onChange(keywords.filter((k) => k !== keyword))}
          >
            <CloseIcon />
          </button>
        </span>
      ))}
      <input
        className="chip-input"
        value={draft}
        placeholder={
          keywords.length === 0 ? "输入关键词，回车或逗号确认" : "继续添加…"
        }
        aria-label="关键词"
        onChange={(event) => {
          const next = event.target.value;
          // Committing on the separator itself means a pasted 「酒店,宾馆,民宿」
          // lands as three chips without the user doing anything.
          if (SEPARATORS.test(next)) commit(next);
          else setDraft(next);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit(draft);
          } else if (
            event.key === "Backspace" &&
            draft === "" &&
            keywords.length > 0
          ) {
            onChange(keywords.slice(0, -1));
          }
        }}
        // A keyword typed but never confirmed is the classic way to lose an
        // edit: the user types 住宿 and clicks 保存, and the chip was never
        // made. Blur commits it.
        onBlur={() => commit(draft)}
      />
    </div>
  );
}
