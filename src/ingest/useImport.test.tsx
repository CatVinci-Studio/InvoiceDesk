/**
 * The two things about an import that are worth pinning down: the fold that
 * turns a stream of events into the tally, and the summary's promises about
 * what it will and will not shout at the user about.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ImportOverlay } from "./ImportOverlay";
import {
  reduceIngest,
  type FileResult,
  type ImportState,
  type Importer,
} from "./useImport";

const IDLE: ImportState = {
  phase: "idle",
  total: 0,
  done: 0,
  currentName: null,
  tally: { imported: 0, needsReview: 0, alreadyImported: 0, failed: 0 },
  problems: [],
  visionCandidates: 0,
};

function fileEvent(overrides: Partial<FileResult> = {}): FileResult {
  return {
    type: "file",
    index: 0,
    name: "发票.pdf",
    status: "imported",
    invoiceId: 1,
    trace: [],
    message: "",
    visionWouldHelp: false,
    ...overrides,
  };
}

describe("reduceIngest", () => {
  it("starts a second import from zero rather than adding to the first", () => {
    const dirty = reduceIngest(
      reduceIngest(IDLE, { type: "started", total: 1 }),
      fileEvent({ status: "failed", message: "读不出来" }),
    );

    const fresh = reduceIngest(dirty, { type: "started", total: 3 });
    expect(fresh.total).toBe(3);
    expect(fresh.done).toBe(0);
    expect(fresh.problems).toHaveLength(0);
    expect(fresh.tally.failed).toBe(0);
  });

  it("keeps the files that need a human and only counts the ones that do not", () => {
    let state = reduceIngest(IDLE, { type: "started", total: 3 });
    state = reduceIngest(state, fileEvent({ index: 0, status: "imported" }));
    state = reduceIngest(state, fileEvent({ index: 1, status: "needsReview" }));
    state = reduceIngest(
      state,
      fileEvent({ index: 2, status: "alreadyImported" }),
    );

    expect(state.done).toBe(3);
    expect(state.tally).toMatchObject({
      imported: 1,
      needsReview: 1,
      alreadyImported: 1,
    });
    // The clean one is a number, not a row - see the note in useImport.ts.
    expect(state.problems.map((p) => p.status)).toEqual([
      "needsReview",
      "alreadyImported",
    ]);
  });

  it("counts the files a vision pass would have finished", () => {
    let state = reduceIngest(IDLE, { type: "started", total: 2 });
    state = reduceIngest(
      state,
      fileEvent({ status: "failed", visionWouldHelp: true }),
    );
    state = reduceIngest(state, fileEvent({ index: 1, status: "failed" }));
    expect(state.visionCandidates).toBe(1);
  });

  it("takes the final counts from the backend, which did the counting", () => {
    let state = reduceIngest(IDLE, { type: "started", total: 2 });
    state = reduceIngest(state, fileEvent({ status: "imported" }));
    state = reduceIngest(state, {
      type: "finished",
      imported: 7,
      needsReview: 2,
      duplicates: 1,
      failed: 0,
    });

    expect(state.phase).toBe("done");
    expect(state.tally.imported).toBe(7);
    expect(state.tally.alreadyImported).toBe(1);
  });
});

function importer(overrides: Partial<Importer>): Importer {
  return {
    ...IDLE,
    phase: "done",
    dragging: false,
    choosing: false,
    pickFiles: () => {},
    chooseFiles: () => {},
    chooseFolder: () => {},
    dismiss: () => {},
    ...overrides,
  };
}

describe("ImportOverlay", () => {
  it("says nothing at all when there is nothing happening", () => {
    const { container } = render(
      <ImportOverlay importer={importer({ phase: "idle" })} />,
    );
    // No jest-dom matchers here: the project has no vitest setup file.
    expect(container.innerHTML).toBe("");
  });

  it("suggests the vision setting once, however many photos there were", () => {
    render(
      <ImportOverlay
        importer={importer({
          total: 4,
          done: 4,
          visionCandidates: 4,
          tally: { imported: 0, needsReview: 0, alreadyImported: 0, failed: 4 },
          problems: [0, 1, 2, 3].map((index) =>
            fileEvent({ index, status: "failed", visionWouldHelp: true }),
          ),
        })}
      />,
    );
    expect(screen.getAllByText("去设置里开启")).toHaveLength(1);
  });

  it("does not dress a re-dropped file up as an error", () => {
    render(
      <ImportOverlay
        importer={importer({
          total: 1,
          done: 1,
          tally: { imported: 0, needsReview: 0, alreadyImported: 1, failed: 0 },
          problems: [fileEvent({ status: "alreadyImported", invoiceId: 9 })],
        })}
      />,
    );
    const badge = screen.getByText("重复文件");
    expect(badge.className).toContain("badge-neutral");
  });
});
