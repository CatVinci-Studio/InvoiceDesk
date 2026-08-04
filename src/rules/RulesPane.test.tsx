/**
 * The pane with the IPC layer stubbed out.
 *
 * Only the behaviour that would be a bug rather than a redesign is asserted:
 * that a rule reads as a sentence without opening anything, that deleting
 * takes two clicks, and that the pane-level actions report a number back
 * instead of silently succeeding - "已补回 3 条内置规则" is the entire point of
 * a button whose effect is otherwise invisible.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { Rule } from "../types";
import { invoices as invoicesApi, rules as rulesApi } from "../ipc";
import { ToastProvider } from "../ui/primitives";
import { RulesPane } from "./RulesPane";

vi.mock("../ipc", () => ({
  errorMessage: (cause: unknown) => String(cause),
  rules: {
    list: vi.fn(),
    save: vi.fn(),
    remove: vi.fn(),
    categories: vi.fn(),
    restoreDefaults: vi.fn(),
    exportJson: vi.fn(),
    importJson: vi.fn(),
    exportToFile: vi.fn(),
    importFromFile: vi.fn(),
  },
  invoices: { reclassifyAll: vi.fn() },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  save: vi.fn(),
}));

const HOTEL: Rule = {
  id: 1,
  name: "税收分类简称：住宿服务",
  category: "住宿",
  priority: 150,
  enabled: true,
  conditions: [{ field: "taxCategory", keywords: ["住宿服务"] }],
};

const SELLER: Rule = {
  id: 2,
  name: "出差住宿（含长包房）",
  category: "住宿",
  priority: 50,
  enabled: false,
  conditions: [{ field: "sellerName", keywords: ["酒店", "宾馆"] }],
};

function setup(rules: Rule[] = [HOTEL, SELLER]) {
  vi.mocked(rulesApi.list).mockResolvedValue(rules);
  const onChanged = vi.fn();
  render(
    <ToastProvider>
      <RulesPane onChanged={onChanged} />
    </ToastProvider>,
  );
  return { onChanged };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("RulesPane", () => {
  it("states the AND/OR semantics on screen rather than in a tooltip", async () => {
    setup();
    expect(await screen.findByText(/同时满足/)).toBeTruthy();
    expect(screen.getByText(/满足任意一条/)).toBeTruthy();
  });

  it("renders a rule as prose, so 为什么归到这个类别 needs no clicking", async () => {
    setup();
    expect(await screen.findByText("税收分类简称")).toBeTruthy();
    expect(screen.getByText("住宿服务")).toBeTruthy();
    // Both keywords of the seller-name rule, with the 或 between them.
    expect(screen.getByText("酒店")).toBeTruthy();
    expect(screen.getByText("宾馆")).toBeTruthy();
    expect(screen.getByText("或")).toBeTruthy();
  });

  /** A built-in's name IS its prose; repeating it would double every row. */
  it("only shows the rule name when the user overrode it", async () => {
    setup();
    expect(await screen.findByText(/出差住宿（含长包房）/)).toBeTruthy();
    expect(screen.queryByText(/记为「税收分类简称：住宿服务」/)).toBeNull();
  });

  it("switches a rule off without deleting it", async () => {
    const { onChanged } = setup();
    vi.mocked(rulesApi.save).mockResolvedValue(1);
    const switches = await screen.findAllByRole("switch");
    fireEvent.click(switches[0]);

    await vi.waitFor(() => expect(rulesApi.save).toHaveBeenCalled());
    expect(vi.mocked(rulesApi.save).mock.calls[0][0].enabled).toBe(false);
    await vi.waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it("asks before deleting a rule", async () => {
    setup();
    vi.mocked(rulesApi.remove).mockResolvedValue(undefined);
    fireEvent.click((await screen.findAllByTitle("删除规则"))[0]);
    expect(rulesApi.remove).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "确认删除" }));
    await vi.waitFor(() => expect(rulesApi.remove).toHaveBeenCalledWith(1));
  });

  it("says how many built-ins came back, since nothing else shows it", async () => {
    setup();
    vi.mocked(rulesApi.restoreDefaults).mockResolvedValue(3);
    fireEvent.click(
      (await screen.findAllByRole("button", { name: "恢复内置规则" }))[0],
    );
    expect(await screen.findByText("已补回 3 条内置规则")).toBeTruthy();
  });

  it("says how many invoices actually moved when the rules are re-run", async () => {
    const { onChanged } = setup();
    vi.mocked(invoicesApi.reclassifyAll).mockResolvedValue(7);
    fireEvent.click(
      await screen.findByRole("button", { name: "重新分类全部发票" }),
    );
    expect(await screen.findByText(/7 张发票的类别有变化/)).toBeTruthy();
    expect(onChanged).toHaveBeenCalled();
  });

  it("writes the rules to the file the user picked", async () => {
    setup();
    vi.mocked(save).mockResolvedValue("/tmp/invoicedesk-规则.json");
    vi.mocked(rulesApi.exportToFile).mockResolvedValue(2);
    fireEvent.click(await screen.findByRole("button", { name: "导出规则" }));

    await vi.waitFor(() =>
      expect(rulesApi.exportToFile).toHaveBeenCalledWith(
        "/tmp/invoicedesk-规则.json",
      ),
    );
    expect(await screen.findByText("已导出 2 条规则")).toBeTruthy();
  });

  /** Closing the dialog is not a failure and must not be reported as one. */
  it("does nothing when the file dialog is cancelled", async () => {
    setup();
    vi.mocked(save).mockResolvedValue(null);
    fireEvent.click(await screen.findByRole("button", { name: "导出规则" }));

    await vi.waitFor(() => expect(save).toHaveBeenCalled());
    expect(rulesApi.exportToFile).not.toHaveBeenCalled();
    expect(screen.queryByText(/已导出/)).toBeNull();
  });

  it("reports how many rules an imported file added", async () => {
    const { onChanged } = setup();
    vi.mocked(open).mockResolvedValue("/tmp/同事的规则.json");
    vi.mocked(rulesApi.importFromFile).mockResolvedValue(12);
    fireEvent.click(await screen.findByRole("button", { name: "导入规则" }));

    expect(await screen.findByText("已导入 12 条规则")).toBeTruthy();
    expect(onChanged).toHaveBeenCalled();
  });

  it("shows the backend's own sentence when an import fails", async () => {
    setup();
    vi.mocked(open).mockResolvedValue("/tmp/坏文件.json");
    vi.mocked(rulesApi.importFromFile).mockRejectedValue(
      "规则文件无法解析：无效的 JSON",
    );
    fireEvent.click(await screen.findByRole("button", { name: "导入规则" }));

    expect(
      await screen.findByText("规则文件无法解析：无效的 JSON"),
    ).toBeTruthy();
  });

  it("offers the built-ins when there are no rules at all", async () => {
    setup([]);
    expect(await screen.findByText("还没有任何分类规则")).toBeTruthy();
  });
});
