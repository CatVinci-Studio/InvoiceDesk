/**
 * 设置 - the AI configuration, the theme, and the about box.
 *
 * The information architecture is Levis's settings panel: a category rail on
 * the left, grouped inset cards on the right, hairlines only between adjacent
 * rows of one card. `Group` and `Row` in `ui/primitives.tsx` already produce
 * that shape, so this file is mostly wording and wiring.
 *
 * ## What this pane is actually for
 *
 * Nothing else in ZhiShui touches the network. Import reads QR codes, PDF
 * text layers and OFD/XML attachments locally; classification runs the user's
 * own rules; the report writer fills a local .xlsx. The two switches in
 * 「发送到 AI 的内容」 are the entire boundary between "this program is
 * offline" and "this program uploads invoices", which is why they come first
 * on the page, before the provider and the key that make them work, and why
 * their sub-text says exactly what leaves the machine rather than something
 * softer.
 *
 * They are two switches and not one because they send genuinely different
 * things - see `src-tauri/src/ai/vision.rs` (the invoice IMAGE, with the
 * buyer, the tax IDs and the amounts printed on it) versus
 * `src-tauri/src/ai/categorize.rs` (票种, 销售方, 项目名称, 备注 - and a test
 * over there asserts no money and no tax id ever joins them). A user who will
 * let a vendor name go out but not a photograph of the invoice is making a
 * coherent choice, and the UI has to be precise enough for them to make it.
 */

import { useEffect, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { appDataDir, join } from "@tauri-apps/api/path";
import { ai, errorMessage, prefs } from "../ipc";
import { DEFAULT_AI_SETTINGS } from "../types";
import type { AiSettings } from "../types";
import {
  Badge,
  Button,
  Group,
  Row,
  Select,
  TextInput,
  Toggle,
  useAsync,
  useToast,
} from "../ui/primitives";
import { ExternalIcon, RefreshIcon } from "../ui/icons";
import {
  APP_NAME,
  APP_VENDOR,
  APP_VERSION,
  LEDGER_FILE_NAME,
} from "./app-info";
import { providerEntry, useProviderCatalog } from "./provider-catalog";
import {
  applyTheme,
  loadThemeChoice,
  THEME_CHOICES,
  THEME_PREF_KEY,
  type ThemeChoice,
} from "./theme";
import "./settings.css";

type Section = "ai" | "appearance" | "about";

const SECTIONS: { id: Section; label: string }[] = [
  { id: "ai", label: "AI 识别" },
  { id: "appearance", label: "外观" },
  { id: "about", label: "关于" },
];

export function SettingsPane() {
  const [section, setSection] = useState<Section>("ai");

  return (
    <div className="settings-pane">
      <nav className="settings-nav">
        {SECTIONS.map((item) => (
          <button
            key={item.id}
            className={`settings-nav-item ${
              section === item.id ? "settings-nav-item-active" : ""
            }`}
            onClick={() => setSection(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>

      <div className="settings-content">
        <div className="settings-sections">
          {section === "ai" && <AiSection />}
          {section === "appearance" && <AppearanceSection />}
          {section === "about" && <AboutSection />}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// AI
// ---------------------------------------------------------------------------

/** The result of 「测试连接」, kept verbatim - see the note where it is set. */
type TestResult = { ok: boolean; text: string };

function AiSection() {
  const toast = useToast();
  const catalog = useProviderCatalog();

  const [settings, setSettings] = useState<AiSettings>(DEFAULT_AI_SETTINGS);
  const [loaded, setLoaded] = useState(false);
  const [keyDraft, setKeyDraft] = useState("");
  const [baseUrlDraft, setBaseUrlDraft] = useState("");
  const [fetchedModels, setFetchedModels] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);

  useEffect(() => {
    ai.getSettings()
      .then((stored) => {
        setSettings(stored);
        setBaseUrlDraft(stored.customBaseUrl ?? "");
      })
      .catch((cause: unknown) => toast(errorMessage(cause), "error"))
      .finally(() => setLoaded(true));
  }, [toast]);

  // Whether a key exists, never what it is: `set_provider_api_key` is
  // write-only by design and there is no command that reads one back. Keyed
  // on the provider so switching services re-checks rather than showing the
  // previous one's status.
  const keyStatus = useAsync(
    () => ai.hasKey(settings.provider),
    [settings.provider],
    false,
  );

  /**
   * Applies one change and stores the whole settings object.
   *
   * Optimistic: a toggle has to move under the finger, and the write is a
   * local SQLite row. If it fails the user is told, and the next time the
   * pane mounts it reads the truth back - which is the honest failure mode,
   * because the backend consults the STORED settings, never this state.
   */
  async function commit(patch: Partial<AiSettings>): Promise<void> {
    const next = { ...settings, ...patch };
    setSettings(next);
    try {
      await ai.setSettings(next);
    } catch (cause) {
      toast(errorMessage(cause), "error");
    }
  }

  // Until the stored settings arrive there is nothing safe to draw: a switch
  // rendered OFF while the stored value is ON is not a loading state, it is a
  // false statement about whether this machine uploads invoices.
  if (!loaded) {
    return <p className="settings-note">正在读取设置…</p>;
  }

  const entry = providerEntry(catalog, settings.provider);
  if (!entry) {
    return (
      <p className="settings-note settings-note-error">服务商列表为空。</p>
    );
  }

  // DeepSeek ships no vision model; "custom" has none in the catalog only
  // because the user supplies it, which is a different statement entirely and
  // must not produce the same warning.
  const providerCannotSeeImages =
    entry.visionModel === null && entry.id !== "custom";
  const models = mergeModels(entry.knownModels, fetchedModels);

  function selectProvider(id: string) {
    // Model ids are provider-local: carrying `qwen-vl-ocr-latest` over to Kimi
    // produces a 404 whose message tells the user nothing about what they did.
    // Clearing both back to null means "use the new provider's default", which
    // is right in every case.
    setFetchedModels([]);
    setTestResult(null);
    setKeyDraft("");
    void commit({ provider: id, visionModel: null, textModel: null });
  }

  async function saveKey() {
    const key = keyDraft.trim();
    if (!key) return;
    try {
      await ai.setKey(settings.provider, key);
      setKeyDraft("");
      keyStatus.reload();
      toast("API Key 已保存");
    } catch (cause) {
      toast(errorMessage(cause), "error");
    }
  }

  async function clearKey() {
    try {
      await ai.clearKey(settings.provider);
      keyStatus.reload();
    } catch (cause) {
      toast(errorMessage(cause), "error");
    }
  }

  /**
   * Both remote calls below resolve their endpoint from the settings row in
   * the database, not from anything passed in - so whatever is on screen has
   * to be committed first or the user tests the previous provider and cannot
   * tell.
   */
  async function withCommittedSettings<T>(run: () => Promise<T>): Promise<T> {
    await ai.setSettings(settings);
    return run();
  }

  async function testConnection() {
    setBusy(true);
    setTestResult(null);
    try {
      const message = await withCommittedSettings(() => ai.testConnection());
      setTestResult({ ok: true, text: message });
    } catch (cause) {
      // Verbatim. Every `Err` on the AI path is already a Chinese sentence
      // written for this reader - 「连接失败：...」, 「还没有配置 AI 服务商…」
      // - and prefixing it with 「操作失败：」 would push the part that says
      // what to do off the end of the line.
      setTestResult({ ok: false, text: errorMessage(cause) });
    } finally {
      setBusy(false);
    }
  }

  async function fetchModels() {
    setBusy(true);
    try {
      const list = await withCommittedSettings(() => ai.listModels());
      setFetchedModels(list);
      toast(`获取到 ${list.length} 个模型`);
    } catch (cause) {
      toast(errorMessage(cause), "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <Group
        title="发送到 AI 的内容"
        hint="两个开关默认都是关闭的。全部关闭时，智税只用二维码、PDF 文本层和本机规则处理发票，不联网。"
      >
        <Row
          label="用 AI 识别拍照/扫描的发票"
          hint="只有离线方式（二维码、PDF 文本层）读不出来的图片才会上传，图片会发送到所选服务商。发票图片上印着购买方名称、税号和金额。"
        >
          <Toggle
            checked={settings.visionEnabled}
            onChange={(checked) => void commit({ visionEnabled: checked })}
          />
        </Row>
        <Row
          label="用 AI 建议费用类别"
          hint="只发送票种、销售方名称、项目名称，不发送金额、税号和购买方。建议只是建议，要你点了「采纳」才会写进规则。"
        >
          <Toggle
            checked={settings.suggestEnabled}
            onChange={(checked) => void commit({ suggestEnabled: checked })}
          />
        </Row>
        {settings.visionEnabled && providerCannotSeeImages && (
          <p className="settings-note settings-note-warn">
            {entry.label}
            没有可用的视觉模型，识别照片时会失败。请换一个服务商，或者只保留下面的分类建议。
          </p>
        )}
      </Group>

      <Group
        title="服务商"
        hint="API Key 保存在配置目录下一个权限 0600 的单独文件里，不写进发票账本 zhishui.db —— 把账本复制走或者交给同事时，不会把钥匙一起带上。"
      >
        <Row
          label="服务商"
          hint="都是国内服务商，接口一致，按价格和识别效果选即可"
        >
          <Select
            value={settings.provider}
            onChange={selectProvider}
            options={catalog.map((item) => ({
              value: item.id,
              label: item.label,
            }))}
          />
        </Row>

        {entry.consoleUrl && (
          <Row label="控制台" hint="在这里创建 API Key，然后粘贴到下面">
            <button
              className="settings-link"
              onClick={() => {
                openUrl(entry.consoleUrl).catch((cause: unknown) =>
                  toast(`打不开链接：${errorMessage(cause)}`, "error"),
                );
              }}
              title={entry.consoleUrl}
            >
              <span className="settings-link-text">{entry.consoleUrl}</span>
              <ExternalIcon />
            </button>
          </Row>
        )}

        <Row
          label="API Key"
          hint={
            entry.keyOptional
              ? "本机服务可以不填"
              : "保存后无法再读出来，只能重新填写"
          }
        >
          {keyStatus.data ? (
            <>
              <Badge tone="ok">已配置</Badge>
              <Button onClick={() => void clearKey()}>清除</Button>
            </>
          ) : (
            <>
              <Badge tone={entry.keyOptional ? "neutral" : "warn"}>
                未配置
              </Badge>
              <input
                type="password"
                className="input settings-key-input"
                value={keyDraft}
                placeholder="粘贴 API Key"
                autoComplete="off"
                spellCheck={false}
                onChange={(event) => setKeyDraft(event.target.value)}
              />
              <Button
                intent="primary"
                disabled={!keyDraft.trim()}
                onClick={() => void saveKey()}
              >
                保存
              </Button>
            </>
          )}
        </Row>

        {entry.baseUrl === null && (
          <Row
            label="接口地址"
            hint="必须兼容 OpenAI Chat Completions，即 {地址}/chat/completions 可用。填到 /v1 为止，不要带 /chat/completions。"
            stacked
          >
            <TextInput
              value={baseUrlDraft}
              onChange={setBaseUrlDraft}
              placeholder="https://例如.example.com/v1"
              mono
              // Committed on blur, not per keystroke: a URL is meaningless
              // half-typed and each commit is a database write.
              onBlur={() => {
                const trimmed = baseUrlDraft.trim();
                if (trimmed === (settings.customBaseUrl ?? "")) return;
                void commit({ customBaseUrl: trimmed || null });
              }}
            />
          </Row>
        )}

        <Row label="测试连接" hint="用当前的地址和 Key 请求一次模型列表">
          <Button disabled={busy} onClick={() => void testConnection()}>
            测试连接
          </Button>
        </Row>
        {testResult && (
          <p
            className={`settings-note ${
              testResult.ok ? "settings-note-ok" : "settings-note-error"
            }`}
          >
            {testResult.text}
          </p>
        )}
      </Group>

      <Group
        title="模型"
        hint="留空就用服务商的默认模型，名字写在选项里。识别发票用视觉模型，分类建议用便宜的文本模型就够。"
      >
        {providerCannotSeeImages ? (
          <p className="settings-note">
            {entry.label}
            的接口没有视觉模型，只能用来建议费用类别，读不了发票照片。
          </p>
        ) : (
          <Row label="识别模型" hint="读取发票照片" stacked>
            <ModelPicker
              // Remounted per provider so the "自定义" escape hatch does not
              // stay open across a switch that just reset the value.
              key={`vision-${entry.id}`}
              value={settings.visionModel}
              models={models}
              defaultModel={entry.visionModel}
              onChange={(value) => void commit({ visionModel: value })}
            />
          </Row>
        )}

        <Row label="分类模型" hint="判断费用类别" stacked>
          <ModelPicker
            key={`text-${entry.id}`}
            value={settings.textModel}
            models={models}
            defaultModel={entry.textModel}
            onChange={(value) => void commit({ textModel: value })}
          />
        </Row>

        {entry.id === "volcengine" && (
          // One string rather than JSX text: line-wrapping inside an element
          // silently eats the spaces around 「——」 and the ep- id.
          <p className="settings-note">
            {"火山方舟按「接入点 ID」调用模型，形如 ep-20240101000000-xxxxx，" +
              "需要先在控制台创建推理接入点，再把它的 ID 填在上面 —— 直接写模型名一般调不通。"}
          </p>
        )}

        {entry.modelsListable && (
          <Row label="模型列表" hint="从服务商拉一份当前可用的模型名">
            <Button disabled={busy} onClick={() => void fetchModels()}>
              <RefreshIcon />
              获取模型列表
            </Button>
          </Row>
        )}
      </Group>
    </>
  );
}

/** Known models first, then anything a live fetch added, without duplicates. */
function mergeModels(known: string[], fetched: string[]): string[] {
  return [...new Set([...known, ...fetched])];
}

const CUSTOM_MODEL = "__custom__";

/**
 * A model field: pick from the catalog, or type one.
 *
 * Empty means "use the provider's default", and the empty option spells that
 * default out - 「默认（qwen-vl-ocr-latest）」 - because an unlabelled blank
 * reads as 未设置, i.e. as something broken, when it is the correct and
 * recommended state. The free-text escape hatch exists for the providers
 * where a catalog list cannot be right: 火山方舟's per-account 接入点 IDs, a
 * self-hosted endpoint, or a model released after this build.
 */
function ModelPicker({
  value,
  models,
  defaultModel,
  onChange,
}: {
  value: string | null;
  models: string[];
  defaultModel: string | null;
  onChange: (value: string | null) => void;
}) {
  const current = value ?? "";
  const [manual, setManual] = useState(
    () => current !== "" && !models.includes(current),
  );

  const textField = (
    <TextInput
      value={current}
      onChange={(next) => onChange(next.trim() || null)}
      placeholder={defaultModel ?? "模型名"}
      mono
    />
  );

  // A provider with no catalog models (自定义接口) has nothing to choose
  // between, and a select holding only 「默认」 and 「自定义」 would be a
  // control that exists to be dismissed.
  if (models.length === 0) return textField;

  const options = [
    {
      value: "",
      label: defaultModel ? `默认（${defaultModel}）` : "默认",
    },
    ...models.map((model) => ({ value: model, label: model })),
    { value: CUSTOM_MODEL, label: "自定义…" },
  ];

  return (
    <div className="settings-model-field">
      <Select
        value={manual ? CUSTOM_MODEL : current}
        options={options}
        onChange={(next) => {
          if (next === CUSTOM_MODEL) {
            // Keep whatever is set; the user is about to edit it, and
            // clearing here would throw away a model name they pasted.
            setManual(true);
            return;
          }
          setManual(false);
          onChange(next || null);
        }}
      />
      {manual && textField}
    </div>
  );
}

// ---------------------------------------------------------------------------
// 外观
// ---------------------------------------------------------------------------

function AppearanceSection() {
  const toast = useToast();
  const [choice, setChoice] = useState<ThemeChoice>("system");

  // Restoring here rather than at startup is a known gap - see `restoreTheme`
  // in theme.ts. Applying on mount at least means the pane never shows a
  // choice that disagrees with the window behind it.
  useEffect(() => {
    void loadThemeChoice().then((stored) => {
      setChoice(stored);
      applyTheme(stored);
    });
  }, []);

  function select(next: ThemeChoice) {
    setChoice(next);
    applyTheme(next);
    prefs
      .set(THEME_PREF_KEY, next)
      .catch((cause: unknown) => toast(errorMessage(cause), "error"));
  }

  return (
    <Group
      title="外观"
      hint="「跟随系统」会跟着 macOS / Windows 的浅色深色设置切换。"
    >
      <Row label="主题" hint="只影响界面配色，不影响导出的报销表">
        <Select value={choice} onChange={select} options={THEME_CHOICES} />
      </Row>
    </Group>
  );
}

// ---------------------------------------------------------------------------
// 关于
// ---------------------------------------------------------------------------

function AboutSection() {
  const toast = useToast();

  // Resolved rather than described: "应用数据目录" is not something a user can
  // type into Finder, and the whole point of showing it is that they can back
  // the file up.
  const ledger = useAsync<string | null>(
    async () => {
      try {
        return await join(await appDataDir(), LEDGER_FILE_NAME);
      } catch {
        return null;
      }
    },
    [],
    null,
  );
  const path = ledger.data;

  return (
    <>
      <Group title="关于">
        <Row label={APP_NAME} hint={APP_VENDOR}>
          <span className="tnum">{APP_VERSION}</span>
        </Row>
        {/* String literals, not JSX text: prettier re-wraps long Chinese
            lines, and every wrap point becomes a space in the rendered
            sentence - which in a paragraph with no word spaces shows up as a
            gap in the middle of a clause. */}
        <p className="settings-about">
          {"智税批量识别增值税电子发票（PDF / OFD）与纸质发票照片，" +
            "自动分类、查重，并导出报销明细表。"}
        </p>
        <p className="settings-about">
          {"所有发票、识别结果和分类规则都保存在这台电脑上，不经过任何服务器。" +
            "只有在你打开上面「AI 识别」里的开关之后，对应的内容才会发送到你自己配置的服务商；" +
            "两个开关都关闭时，除了检查更新，智税不会向外发起任何网络请求。"}
        </p>
      </Group>

      <Group
        title="数据位置"
        hint="所有发票记录、分类规则和报销单都在这一个文件里。备份它就等于备份了全部数据；把它删掉，一切从头开始。"
      >
        <Row label="发票账本" hint={LEDGER_FILE_NAME} stacked>
          <div className="settings-path-row">
            <code className="settings-path">
              {path ?? "（无法读取应用数据目录）"}
            </code>
            {path && (
              <Button
                onClick={() => {
                  revealItemInDir(path).catch((cause: unknown) =>
                    toast(`打不开所在文件夹：${errorMessage(cause)}`, "error"),
                  );
                }}
              >
                在文件管理器中显示
              </Button>
            )}
          </div>
        </Row>
      </Group>
    </>
  );
}
