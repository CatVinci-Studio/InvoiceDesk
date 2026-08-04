import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { InvoiceRow } from "../types";
import { InvoiceTable } from "./InvoiceTable";
import { DEFAULT_SORT } from "./rows";

function row(overrides: Partial<InvoiceRow>): InvoiceRow {
  return {
    id: 1,
    number: "24312000000012345678",
    issuedOn: "2024-03-01",
    kind: "数电票（普通）",
    sellerName: "某某酒店",
    totalCents: 106000,
    taxCents: 6000,
    category: "差旅",
    categoryRule: "住宿",
    minConfidence: 1,
    issueCount: 0,
    reviewed: false,
    sourcePath: "/tmp/a.pdf",
    duplicateOf: [],
    ...overrides,
  };
}

const clean = row({ id: 1 });
const doubtful = row({ id: 2, minConfidence: 0.75 });
const duplicated = row({ id: 3, duplicateOf: [9] });
const twin = row({ id: 9, sellerName: "某某酒店", issuedOn: "2024-02-11" });

function renderTable(
  overrides: Partial<Parameters<typeof InvoiceTable>[0]> = {},
) {
  // The spies are kept as their own bindings rather than read back off the
  // props object, so they stay typed as mocks whatever the overrides do.
  const onSortChange = vi.fn();
  const onSelectedChange = vi.fn();
  const onOpen = vi.fn();
  const onCategoryChange = vi.fn();
  const props = {
    rows: [clean, doubtful, duplicated],
    sort: DEFAULT_SORT,
    onSortChange,
    selected: new Set<number>(),
    onSelectedChange,
    activeId: null,
    onOpen,
    categories: ["差旅", "办公"],
    onCategoryChange,
    duplicateIndex: new Map([[9, twin]]),
    ...overrides,
  };
  return {
    ...render(<InvoiceTable {...props} />),
    props,
    onSortChange,
    onSelectedChange,
    onOpen,
  };
}

function statusCellOf(id: number) {
  const tr = document.querySelector(`[data-row-id="${id}"]`) as HTMLElement;
  return tr.querySelector(".cell-status") as HTMLElement;
}

describe("status column", () => {
  /** The colour budget: a clean row is silent. A green tick on every good row
   *  is what makes the one amber badge among them invisible. */
  it("says nothing at all about a row that is fine", () => {
    renderTable();
    expect(statusCellOf(1).textContent).toBe("");
  });

  it("marks low confidence 待复核 and a shared number 疑似重复", () => {
    renderTable();
    expect(within(statusCellOf(2)).getByText("待复核")).toBeTruthy();
    expect(within(statusCellOf(3)).getByText("疑似重复")).toBeTruthy();
  });
});

describe("duplicate link", () => {
  it("names the other row, including its date and amount", () => {
    renderTable();
    fireEvent.click(screen.getByText("疑似重复"));
    expect(screen.getByText("2024/02/11")).toBeTruthy();
    expect(screen.getByText("¥1,060.00")).toBeTruthy();
  });

  it("opens the twin, which works whether or not the filter shows it", () => {
    const { onOpen } = renderTable();
    fireEvent.click(screen.getByText("疑似重复"));
    fireEvent.click(screen.getByText("2024/02/11"));
    expect(onOpen).toHaveBeenCalledWith(9);
  });
});

describe("selection", () => {
  it("opens on a row click but not on a checkbox click", () => {
    const { onOpen, onSelectedChange } = renderTable();
    const rowElement = document.querySelector(
      '[data-row-id="2"]',
    ) as HTMLElement;
    fireEvent.click(rowElement);
    expect(onOpen).toHaveBeenCalledWith(2);

    onOpen.mockClear();
    fireEvent.click(rowElement.querySelector('input[type="checkbox"]')!);
    expect(onOpen).not.toHaveBeenCalled();
    expect(onSelectedChange).toHaveBeenCalledWith(new Set([2]));
  });

  /** Shift means "extend", never "open" - the whole point of a range is that
   *  you can see all of it afterwards. */
  it("extends a range on shift-click without opening anything", () => {
    const { onOpen, onSelectedChange } = renderTable();
    fireEvent.click(document.querySelector('[data-row-id="1"]') as HTMLElement);
    fireEvent.click(
      document.querySelector('[data-row-id="3"]') as HTMLElement,
      {
        shiftKey: true,
      },
    );
    expect(onSelectedChange).toHaveBeenCalledWith(new Set([1, 2, 3]));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("select-all in the header covers exactly the visible rows", () => {
    const { onSelectedChange } = renderTable();
    fireEvent.click(screen.getByTitle("全选当前列表"));
    expect(onSelectedChange).toHaveBeenCalledWith(new Set([1, 2, 3]));
  });

  it("clears only the visible rows, leaving a selection made elsewhere", () => {
    const { onSelectedChange } = renderTable({
      selected: new Set([1, 2, 3, 77]),
    });
    fireEvent.click(screen.getByTitle("全选当前列表"));
    expect(onSelectedChange).toHaveBeenCalledWith(new Set([77]));
  });
});

describe("sorting", () => {
  it("flips direction on the active column and starts new ones descending", () => {
    const { props, rerender, onSortChange } = renderTable();
    fireEvent.click(screen.getByText("开票日期"));
    expect(onSortChange).toHaveBeenCalledWith({ key: "date", desc: false });

    rerender(<InvoiceTable {...props} sort={{ key: "date", desc: false }} />);
    fireEvent.click(screen.getByText("销售方名称"));
    expect(onSortChange).toHaveBeenCalledWith({ key: "seller", desc: true });
  });
});
