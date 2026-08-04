/**
 * The promises this pane makes.
 *
 * Two of these tests assert on exact Chinese sentences, which normally makes
 * a brittle test. They are here on purpose. The sub-text under each AI switch
 * is the only place the app states what leaves the machine, and the precise
 * difference between the two - a picture of the whole invoice versus four
 * text fields with no amounts, no tax IDs and no buyer - is the reason a user
 * can reasonably allow one and refuse the other. Rewording either into
 * something vaguer is a product change, not a copy edit, and it should have
 * to break a test to happen.
 *
 * The third asserts both switches render off, which is the same promise
 * `defaults_keep_both_network_features_off` makes on the Rust side.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SettingsPane } from "./SettingsPane";
import { applyTheme } from "./theme";

const backend = vi.hoisted(() => ({
  aiSettings: {
    visionEnabled: false,
    suggestEnabled: false,
    provider: "qwen",
    visionModel: null,
    textModel: null,
    customBaseUrl: null,
  } as Record<string, unknown>,
  saved: [] as Record<string, unknown>[],
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {},
  invoke: vi.fn(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "get_ai_settings":
        return backend.aiSettings;
      case "set_ai_settings":
        backend.saved.push(args?.settings as Record<string, unknown>);
        backend.aiSettings = args?.settings as Record<string, unknown>;
        return null;
      // Resolving null is what the dev shim does; the pane has to fall back to
      // its own catalog copy rather than rendering an empty picker.
      case "list_providers":
        return null;
      case "provider_api_key_status":
        return false;
      case "get_preference":
        return null;
      default:
        return null;
    }
  }),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(async () => {}),
  revealItemInDir: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: vi.fn(
    async () =>
      "/Users/someone/Library/Application Support/com.chengaoshen.zhishui",
  ),
  join: vi.fn(async (...parts: string[]) => parts.join("/")),
}));

async function renderPane() {
  render(<SettingsPane />);
  // The pane draws nothing until the stored settings land - see the guard in
  // AiSection for why a half-rendered switch is not an acceptable placeholder.
  await waitFor(() => expect(screen.getAllByRole("switch")).toHaveLength(2));
}

describe("SettingsPane", () => {
  it("says plainly what the vision switch uploads", async () => {
    await renderPane();
    expect(document.body.textContent).toContain(
      "只有离线方式（二维码、PDF 文本层）读不出来的图片才会上传，图片会发送到所选服务商。",
    );
  });

  it("says plainly, and narrowly, what the suggestion switch sends", async () => {
    await renderPane();
    expect(document.body.textContent).toContain(
      "只发送票种、销售方名称、项目名称，不发送金额、税号和购买方。",
    );
  });

  it("shows both switches off", async () => {
    await renderPane();
    for (const toggle of screen.getAllByRole("switch")) {
      expect(toggle.getAttribute("aria-checked")).toBe("false");
    }
  });

  /** Turning a switch on has to reach the ledger, not just the React tree -
   *  the extraction pipeline reads the stored row, never this component. */
  it("stores the vision switch when it is turned on", async () => {
    await renderPane();
    backend.saved.length = 0;
    fireEvent.click(screen.getAllByRole("switch")[0]);
    await waitFor(() => expect(backend.saved).toHaveLength(1));
    expect(backend.saved[0].visionEnabled).toBe(true);
    expect(backend.saved[0].suggestEnabled).toBe(false);
  });

  /** Falls back to the bundled catalog when `list_providers` gives nothing. */
  it("still offers real providers without a backend catalog", async () => {
    await renderPane();
    expect(document.body.textContent).toContain("阿里云百炼（通义千问）");
  });
});

/**
 * The theme contract with App.css: `system` must leave an attribute that is
 * neither "light" nor "dark", so `:root:not([data-theme="light"])` inside the
 * dark media query still matches and `:root[data-theme="dark"]` does not.
 */
describe("applyTheme", () => {
  it("maps the three choices onto data-theme", () => {
    applyTheme("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    applyTheme("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    applyTheme("system");
    expect(document.documentElement.getAttribute("data-theme")).toBe("");
  });
});
