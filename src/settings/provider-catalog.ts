/**
 * A TypeScript mirror of the Rust provider catalog.
 *
 * The list of record lives in `src-tauri/src/ai/catalog.rs` - read its module
 * doc for why every entry is a Chinese service and what each field promises.
 * The copy below is **kept in sync by hand**; comments on both sides point
 * here, and `provider-catalog.test.ts` parses the Rust file and fails the
 * build if the two drift apart. That test is the only reason a hand-kept copy
 * is acceptable at all.
 *
 * Why keep a copy when `ai.providers()` returns the real thing:
 *
 * - It is the first paint. The provider picker renders with real labels
 *   instead of an empty select that fills in a tick later.
 * - It is the fallback. In the browser dev server there is no Tauri host and
 *   `invoke` never resolves to a catalog; without this the settings pane
 *   would be unusable outside a packaged build.
 *
 * The live call still wins whenever it answers, so a build whose backend
 * knows about a provider this file has not caught up with shows it anyway.
 */

import { useEffect, useState } from "react";
import { ai } from "../ipc";
import type { ProviderCatalogEntry } from "../types";

export const FALLBACK_PROVIDER_CATALOG: ProviderCatalogEntry[] = [
  {
    id: "qwen",
    label: "阿里云百炼（通义千问）",
    consoleUrl: "https://bailian.console.aliyun.com/",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    visionModel: "qwen-vl-ocr-latest",
    textModel: "qwen-flash",
    keyOptional: false,
    modelsListable: true,
    knownModels: [
      "qwen-vl-ocr-latest",
      "qwen-vl-max-latest",
      "qwen-vl-plus-latest",
      "qwen-flash",
      "qwen-plus",
    ],
  },
  {
    id: "zhipu",
    label: "智谱 GLM",
    consoleUrl: "https://bigmodel.cn/usercenter/apikeys",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    visionModel: "glm-4v-flash",
    textModel: "glm-4-flash",
    keyOptional: false,
    modelsListable: false,
    knownModels: ["glm-4v-flash", "glm-4v-plus", "glm-4-flash", "glm-4-air"],
  },
  {
    id: "volcengine",
    label: "火山方舟（豆包）",
    consoleUrl: "https://console.volcengine.com/ark",
    baseUrl: "https://ark.cn-beijing.volces.com/api/v3",
    visionModel: "doubao-vision-pro-32k",
    textModel: "doubao-lite-32k",
    keyOptional: false,
    modelsListable: false,
    knownModels: ["doubao-vision-pro-32k", "doubao-lite-32k"],
  },
  {
    id: "moonshot",
    label: "月之暗面 Kimi",
    consoleUrl: "https://platform.moonshot.cn/console/api-keys",
    baseUrl: "https://api.moonshot.cn/v1",
    visionModel: "moonshot-v1-8k-vision-preview",
    textModel: "moonshot-v1-8k",
    keyOptional: false,
    modelsListable: true,
    knownModels: [
      "moonshot-v1-8k-vision-preview",
      "moonshot-v1-8k",
      "moonshot-v1-32k",
    ],
  },
  {
    id: "stepfun",
    label: "阶跃星辰 StepFun",
    consoleUrl: "https://platform.stepfun.com/",
    baseUrl: "https://api.stepfun.com/v1",
    visionModel: "step-1v-8k",
    textModel: "step-1-8k",
    keyOptional: false,
    modelsListable: true,
    knownModels: ["step-1v-8k", "step-1v-32k", "step-1-8k"],
  },
  {
    id: "hunyuan",
    label: "腾讯混元",
    consoleUrl: "https://console.cloud.tencent.com/hunyuan/api-key",
    baseUrl: "https://api.hunyuan.cloud.tencent.com/v1",
    visionModel: "hunyuan-vision",
    textModel: "hunyuan-lite",
    keyOptional: false,
    modelsListable: false,
    knownModels: ["hunyuan-vision", "hunyuan-lite", "hunyuan-standard"],
  },
  {
    id: "minimax",
    label: "MiniMax",
    consoleUrl: "https://platform.minimaxi.com/user-center/basic-information",
    baseUrl: "https://api.minimaxi.com/v1",
    visionModel: "MiniMax-VL-01",
    textModel: "MiniMax-Text-01",
    keyOptional: false,
    modelsListable: false,
    knownModels: ["MiniMax-VL-01", "MiniMax-Text-01"],
  },
  {
    id: "siliconflow",
    label: "硅基流动 SiliconFlow",
    consoleUrl: "https://cloud.siliconflow.cn/account/ak",
    baseUrl: "https://api.siliconflow.cn/v1",
    visionModel: "Qwen/Qwen2.5-VL-72B-Instruct",
    textModel: "Qwen/Qwen2.5-7B-Instruct",
    keyOptional: false,
    modelsListable: true,
    knownModels: [
      "Qwen/Qwen2.5-VL-72B-Instruct",
      "Qwen/Qwen2.5-VL-32B-Instruct",
      "Qwen/Qwen2.5-7B-Instruct",
    ],
  },
  {
    id: "deepseek",
    label: "深度求索 DeepSeek",
    consoleUrl: "https://platform.deepseek.com/api_keys",
    baseUrl: "https://api.deepseek.com",
    visionModel: null,
    textModel: "deepseek-chat",
    keyOptional: false,
    modelsListable: true,
    knownModels: ["deepseek-chat", "deepseek-reasoner"],
  },
  {
    id: "ollama",
    label: "Ollama（本机）",
    consoleUrl: "https://ollama.com/",
    baseUrl: "http://localhost:11434/v1",
    visionModel: "qwen2.5vl:7b",
    textModel: "qwen2.5:7b",
    keyOptional: true,
    modelsListable: true,
    knownModels: ["qwen2.5vl:7b", "qwen2.5vl:3b", "qwen2.5:7b"],
  },
  {
    id: "custom",
    label: "自定义接口",
    consoleUrl: "",
    baseUrl: null,
    visionModel: null,
    textModel: null,
    keyOptional: true,
    modelsListable: true,
    knownModels: [],
  },
];

/**
 * The catalog: the mirror above, replaced by the backend's list once it
 * answers.
 *
 * The rejection is swallowed on purpose. The only ways this call fails are a
 * missing Tauri host (dev server) and a backend older than this bundle, and
 * in both cases the fallback is already showing something correct - a toast
 * would report a problem the user cannot act on, over a UI that is working.
 */
export function useProviderCatalog(): ProviderCatalogEntry[] {
  const [catalog, setCatalog] = useState<ProviderCatalogEntry[]>(
    FALLBACK_PROVIDER_CATALOG,
  );

  useEffect(() => {
    ai.providers()
      .then((list) => {
        if (list?.length) setCatalog(list);
      })
      .catch(() => {});
  }, []);

  return catalog;
}

/**
 * The entry for `id`, or the first one.
 *
 * A stored provider that no longer exists in the catalog (renamed, dropped)
 * would otherwise leave the pane with nothing to render and no way back -
 * every control that could change the provider is inside the very block that
 * failed to draw. Falling back to a real entry keeps the picker usable; the
 * stored id is left alone until the user picks something, so nothing is
 * silently rewritten behind their back.
 */
export function providerEntry(
  catalog: ProviderCatalogEntry[],
  id: string,
): ProviderCatalogEntry | undefined {
  return catalog.find((entry) => entry.id === id) ?? catalog[0];
}
