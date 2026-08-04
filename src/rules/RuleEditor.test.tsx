/**
 * The editor is pure props in, rule out - no IPC - which is exactly why these
 * tests can be plain vitest with no Tauri host anywhere near them.
 *
 * What is worth testing here is the part a user would report as a bug: a rule
 * that saves but never matches. Both ways of building one (no conditions, or a
 * condition with no keywords) have to be unreachable through the UI.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { RuleEditor } from "./RuleEditor";
import { blankRule } from "./ruleText";

function setup(categories: string[] = ["住宿", "餐饮"]) {
  const onSave = vi.fn().mockResolvedValue(undefined);
  const onCancel = vi.fn();
  render(
    <RuleEditor
      rule={blankRule()}
      categories={categories}
      onSave={onSave}
      onCancel={onCancel}
    />,
  );
  return { onSave, onCancel };
}

function saveButton() {
  return screen.getByRole("button", { name: "保存" }) as HTMLButtonElement;
}

function typeCategory(value: string) {
  fireEvent.change(screen.getByPlaceholderText("例如 差旅费"), {
    target: { value },
  });
}

function addKeyword(value: string, index = 0) {
  const input = screen.getAllByLabelText("关键词")[index];
  fireEvent.change(input, { target: { value } });
  fireEvent.keyDown(input, { key: "Enter" });
}

describe("RuleEditor", () => {
  it("will not save a rule whose condition has no keywords", () => {
    setup();
    typeCategory("住宿");
    expect(saveButton().disabled).toBe(true);
    expect(screen.getByText(/关键词/)).toBeTruthy();
  });

  it("will not save a rule with no conditions at all", () => {
    setup();
    typeCategory("住宿");
    addKeyword("住宿服务");
    expect(saveButton().disabled).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "删除这个条件" }));
    expect(saveButton().disabled).toBe(true);
    expect(screen.getByText(/至少要有一个条件/)).toBeTruthy();
  });

  it("names the rule after its conditions unless the user says otherwise", async () => {
    const { onSave } = setup();
    typeCategory("住宿");
    addKeyword("住宿服务");
    fireEvent.click(saveButton());

    await vi.waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0]).toMatchObject({
      category: "住宿",
      name: "税收分类简称：住宿服务",
      priority: 150,
      conditions: [{ field: "taxCategory", keywords: ["住宿服务"] }],
    });
  });

  it("keeps a name the user typed", async () => {
    const { onSave } = setup();
    typeCategory("住宿");
    addKeyword("住宿服务");
    fireEvent.change(screen.getByPlaceholderText("税收分类简称：住宿服务"), {
      target: { value: "出差住宿" },
    });
    fireEvent.click(saveButton());

    await vi.waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0].name).toBe("出差住宿");
  });

  /** Pasting a comma-separated list is how anyone with an existing list of
   *  vendors will fill this in. */
  it("splits pasted keywords on any of the usual separators", () => {
    setup();
    const input = screen.getAllByLabelText("关键词")[0];
    fireEvent.change(input, { target: { value: "酒店,宾馆、民宿" } });
    expect(screen.getByText("酒店")).toBeTruthy();
    expect(screen.getByText("宾馆")).toBeTruthy();
    expect(screen.getByText("民宿")).toBeTruthy();
  });

  it("lets a priority band be picked instead of typed", async () => {
    const { onSave } = setup();
    typeCategory("市内交通");
    addKeyword("出租车票");
    fireEvent.click(screen.getByRole("button", { name: /销售方兜底/ }));
    fireEvent.click(saveButton());

    await vi.waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave.mock.calls[0][0].priority).toBe(50);
  });

  it("makes it explicit when a typed category does not exist yet", () => {
    setup(["住宿"]);
    typeCategory("团建");
    expect(screen.getByText(/将新建类别/)).toBeTruthy();
  });
});
