import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CategoryTotal, InvoiceFilter, InvoiceRow } from "../types";
import { ToastProvider } from "../ui/primitives";
import { InvoicesPane } from "./InvoicesPane";

const list = vi.fn<(filter: InvoiceFilter) => Promise<InvoiceRow[]>>();
const totals = vi.fn<(filter: InvoiceFilter) => Promise<CategoryTotal[]>>();

vi.mock("../ipc", () => ({
  invoices: {
    list: (filter: InvoiceFilter) => list(filter),
    totals: (filter: InvoiceFilter) => totals(filter),
    setCategory: vi.fn(async () => {}),
    remove: vi.fn(async () => {}),
  },
  rules: { categories: async () => ["差旅", "办公"] },
  errorMessage: (error: unknown) => String(error),
}));

const rows: InvoiceRow[] = [
  {
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
    reviewed: true,
    sourcePath: "/tmp/a.pdf",
    duplicateOf: [],
  },
];

beforeEach(() => {
  list.mockReset();
  totals.mockReset();
  list.mockResolvedValue(rows);
  totals.mockResolvedValue([
    { category: "差旅", count: 1, totalCents: 106000, taxCents: 6000 },
  ]);
});

function renderPane(scope: "all" | "review" | "duplicates") {
  return render(
    <ToastProvider>
      <InvoicesPane scope={scope} onChanged={vi.fn()} />
    </ToastProvider>,
  );
}

/** The one filter both the table and the status bar are built from. */
function filterOf(mock: { mock: { calls: [InvoiceFilter][] } }) {
  // The pane also asks for a ledger-wide duplicate index, which is a separate
  // and deliberately unfiltered query; the one under test is the one carrying
  // the toolbar's own fields.
  return mock.mock.calls
    .map(([filter]) => filter)
    .find((filter) => "search" in filter);
}

describe("scope → filter", () => {
  it("leaves 全部发票 unconstrained, with both flags spelled out", async () => {
    renderPane("all");
    await waitFor(() => expect(list).toHaveBeenCalled());
    // Sent explicitly rather than omitted: `db::Filter` declares them as
    // plain `bool`, which serde will not default for a missing field.
    expect(list).toHaveBeenCalledWith(
      expect.objectContaining({
        needsReviewOnly: false,
        duplicatesOnly: false,
      }),
    );
  });

  it("forces the filter the sidebar promised, and hides the toggle", async () => {
    renderPane("review");
    await waitFor(() =>
      expect(list).toHaveBeenCalledWith(
        expect.objectContaining({
          needsReviewOnly: true,
          duplicatesOnly: false,
        }),
      ),
    );
    expect(screen.queryByText("只看待复核")).toBeNull();
  });

  it("does the same for 疑似重复", async () => {
    renderPane("duplicates");
    await waitFor(() =>
      expect(list).toHaveBeenCalledWith(
        expect.objectContaining({
          needsReviewOnly: false,
          duplicatesOnly: true,
        }),
      ),
    );
  });
});

describe("status bar", () => {
  /** The summary and the table must never describe different sets of rows. */
  it("totals the same filter the table was built from", async () => {
    renderPane("all");
    await screen.findByText("¥1,060.00");
    expect(screen.getByText("¥60.00")).toBeTruthy();
    expect(filterOf(totals)).toEqual(filterOf(list));
  });

  it("offers the toggle only in 全部发票", async () => {
    renderPane("all");
    expect(await screen.findByText("只看待复核")).toBeTruthy();
  });
});
