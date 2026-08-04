/**
 * Guards the two hand-kept copies this module depends on.
 *
 * `provider-catalog.ts` mirrors `src-tauri/src/ai/catalog.rs` and `app-info.ts`
 * mirrors the version in `tauri.conf.json`. Both are duplicated deliberately
 * (see each file's doc comment), and both fail quietly when they drift: the
 * settings pane would show a stale model list, or an about box would name a
 * version the user does not have. Neither surfaces as a crash, so it has to
 * surface here.
 *
 * The parsing below is deliberately literal - it reads the Rust source as
 * text rather than trying to be a Rust parser. If the catalog's formatting
 * ever changes enough to break it, the assertion at the top (that we found as
 * many entries as the TS side has) fails loudly rather than silently checking
 * nothing.
 */

import { describe, expect, it } from "vitest";
// `?raw` rather than `node:fs`: this project has no @types/node, and Vite's
// own text import is the one way to read a file from a test here that both
// `tsc` and the jsdom test environment agree on.
import catalogSource from "../../src-tauri/src/ai/catalog.rs?raw";
import tauriConf from "../../src-tauri/tauri.conf.json?raw";
import packageJson from "../../package.json?raw";
import { FALLBACK_PROVIDER_CATALOG } from "./provider-catalog";
import { APP_VERSION } from "./app-info";
import type { ProviderCatalogEntry } from "../types";

function readRustCatalog(): ProviderCatalogEntry[] {
  const source = catalogSource;

  const start = source.indexOf(
    "PROVIDER_CATALOG: &[ProviderCatalogEntry] = &[",
  );
  expect(start).toBeGreaterThan(-1);
  const end = source.indexOf("\n];", start);
  expect(end).toBeGreaterThan(start);
  const body = source.slice(start, end);

  return body
    .split("ProviderCatalogEntry {")
    .slice(1)
    .map((block) => ({
      id: str(block, "id") ?? "",
      label: str(block, "label") ?? "",
      consoleUrl: str(block, "console_url") ?? "",
      baseUrl: optional(block, "base_url"),
      visionModel: optional(block, "vision_model"),
      textModel: optional(block, "text_model"),
      keyOptional: bool(block, "key_optional"),
      modelsListable: bool(block, "models_listable"),
      knownModels: list(block, "known_models"),
    }));
}

function str(block: string, field: string): string | null {
  return new RegExp(`\\b${field}: "([^"]*)"`).exec(block)?.[1] ?? null;
}

function optional(block: string, field: string): string | null {
  const match = new RegExp(`\\b${field}: (None|Some\\("([^"]*)"\\))`).exec(
    block,
  );
  return match?.[2] ?? null;
}

function bool(block: string, field: string): boolean {
  return new RegExp(`\\b${field}: true`).test(block);
}

function list(block: string, field: string): string[] {
  const match = new RegExp(`\\b${field}: &\\[([\\s\\S]*?)\\]`).exec(block);
  return [...(match?.[1] ?? "").matchAll(/"([^"]*)"/g)].map((m) => m[1]);
}

describe("provider catalog", () => {
  const rust = readRustCatalog();

  it("was parsed out of the Rust source at all", () => {
    expect(rust.length).toBe(FALLBACK_PROVIDER_CATALOG.length);
    expect(rust.length).toBeGreaterThan(1);
  });

  /** Order matters: it is the order the picker offers, and Qwen leads because
   *  `qwen-vl-ocr` is purpose-built for dense Chinese document text. */
  it("mirrors the Rust catalog entry for entry, in order", () => {
    expect(FALLBACK_PROVIDER_CATALOG).toEqual(rust);
  });

  /** The pane hides 「获取模型列表」 for these, so a wrong flag offers a button
   *  that can only fail. */
  it("agrees about which providers can list their models", () => {
    const listable = rust.filter((e) => e.modelsListable).map((e) => e.id);
    expect(
      FALLBACK_PROVIDER_CATALOG.filter((e) => e.modelsListable).map(
        (e) => e.id,
      ),
    ).toEqual(listable);
  });

  /** The pane tells the user this provider cannot read a photo, which is only
   *  true if both sides agree it has no vision model. */
  it("agrees about which providers have no vision model", () => {
    const blind = rust.filter((e) => e.visionModel === null).map((e) => e.id);
    expect(blind).toContain("deepseek");
    expect(
      FALLBACK_PROVIDER_CATALOG.filter((e) => e.visionModel === null).map(
        (e) => e.id,
      ),
    ).toEqual(blind);
  });
});

describe("app version", () => {
  function versionOf(json: string): string {
    return (JSON.parse(json) as { version: string }).version;
  }

  it("matches the bundle and the package", () => {
    expect(APP_VERSION).toBe(versionOf(tauriConf));
    expect(APP_VERSION).toBe(versionOf(packageJson));
  });
});
