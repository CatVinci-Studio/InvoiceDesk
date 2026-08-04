/**
 * Importing: the window-wide drag target, the file pickers, and the running
 * tally of what came out.
 *
 * The whole module exists to answer one question the moment an import ends:
 * *which of these files still needs me?* Everything else is chrome. That is
 * why the state kept here is asymmetric - a file that read cleanly is only a
 * number, while a file that did not keeps its name, its message and the full
 * trace of which extraction layer was tried. A two-hundred-file folder import
 * would otherwise retain two hundred trace arrays that nothing ever renders.
 *
 * The overlay that draws all of this lives next door in `ImportOverlay.tsx`
 * and is re-exported here, because the shell treats the pair as one thing.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { errorMessage, ingest } from "../ipc";
import type { FileStatus, IngestEvent } from "../types";
import { useToast } from "../ui/primitives";

export { ImportOverlay } from "./ImportOverlay";

// ---------------------------------------------------------------------------
// What the picker accepts
// ---------------------------------------------------------------------------

/**
 * One filter, not four.
 *
 * A native open panel with several filters makes the user pick the right
 * category before they can even see their files, and people do not sort
 * their 发票 folder by "electronic invoice" vs "photo of a receipt" - they
 * have one folder with all of it. The Rust side sniffs the actual bytes
 * anyway (`extract::detect`), so the extension list is a courtesy that keeps
 * unrelated files out of the dialog, not a contract.
 */
const INVOICE_FILTER = {
  name: "发票文件",
  extensions: [
    "pdf",
    "ofd",
    "xml",
    "zip",
    "jpg",
    "jpeg",
    "png",
    "bmp",
    "webp",
    "tif",
    "tiff",
    "heic",
  ],
};

/** Shown wherever the user is about to choose files. Kept in one place so the
 *  drop hint and the chooser cannot drift apart. */
export const SUPPORTED_FORMATS = "PDF / OFD / XML / 图片 / ZIP 压缩包";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/** One file's outcome, exactly as the backend reported it. */
export type FileResult = Extract<IngestEvent, { type: "file" }>;

/** How many files landed in each bucket. */
export type Tally = Record<FileStatus, number>;

export type ImportPhase = "idle" | "running" | "done";

export interface ImportState {
  phase: ImportPhase;
  /** Files the backend found after walking folders and unpacking zips - so
   *  it is usually larger than the number of paths that were dropped. */
  total: number;
  done: number;
  /** The file currently being reported on, for the progress line. */
  currentName: string | null;
  tally: Tally;
  /** Only the files that were NOT cleanly imported - see the module note. */
  problems: FileResult[];
  /** How many files the vision fallback could have finished, had it been on. */
  visionCandidates: number;
}

const EMPTY_TALLY: Tally = {
  imported: 0,
  needsReview: 0,
  alreadyImported: 0,
  failed: 0,
};

const IDLE_STATE: ImportState = {
  phase: "idle",
  total: 0,
  done: 0,
  currentName: null,
  tally: EMPTY_TALLY,
  problems: [],
  visionCandidates: 0,
};

/**
 * Folds one event into the running state.
 *
 * Pulled out of the hook and exported so it can be tested without a Tauri
 * webview underneath it - the ordering rules here (a `started` resets, a
 * `finished` overrides the running tally) are the part that would silently
 * rot.
 */
export function reduceIngest(
  state: ImportState,
  event: IngestEvent,
): ImportState {
  switch (event.type) {
    case "started":
      // A second import must not add to the first one's numbers.
      return { ...IDLE_STATE, phase: "running", total: event.total };

    case "file": {
      const tally: Tally = { ...state.tally };
      tally[event.status] += 1;
      return {
        ...state,
        done: state.done + 1,
        currentName: event.name,
        tally,
        visionCandidates:
          state.visionCandidates + (event.visionWouldHelp ? 1 : 0),
        problems:
          event.status === "imported"
            ? state.problems
            : [...state.problems, event],
      };
    }

    case "finished":
      // The backend counted these itself while writing to the ledger, so they
      // are the authority - the incremental tally above exists only so the
      // numbers move while the import runs.
      return {
        ...state,
        phase: "done",
        currentName: null,
        tally: {
          imported: event.imported,
          needsReview: event.needsReview,
          alreadyImported: event.duplicates,
          failed: event.failed,
        },
      };
  }
}

// ---------------------------------------------------------------------------
// The hook
// ---------------------------------------------------------------------------

export interface Importer extends ImportState {
  /** A drag is currently over the window. */
  dragging: boolean;
  /** The small "文件 or 文件夹" chooser is open. */
  choosing: boolean;
  /** The shell's 导入发票 button. Opens the chooser - see the note below. */
  pickFiles: () => void;
  /** Straight to the native file dialog. */
  chooseFiles: () => void;
  /** Straight to the native folder dialog. */
  chooseFolder: () => void;
  /** Closes the chooser or the finished summary, whichever is up. */
  dismiss: () => void;
}

/**
 * Owns an import from the first drop to the summary the user dismisses.
 *
 * `onFinished` fires once per import, after the last file, so the shell can
 * re-read its counts. It fires even when the import failed outright, because
 * the ledger may still have gained rows before whatever went wrong.
 */
export function useImport(onFinished: () => void): Importer {
  const [state, setState] = useState<ImportState>(IDLE_STATE);
  const [dragging, setDragging] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const toast = useToast();

  // Held in a ref so `start` never changes identity: the drag-and-drop
  // listener is registered against it exactly once, and a shell that rebuilds
  // its refresh callback on some unrelated render must not tear that listener
  // down - possibly mid-drag. Synced in an effect rather than during render
  // because the only reader is an import that is already several awaits past
  // the commit, so there is nothing to gain from being early.
  const finished = useRef(onFinished);
  useEffect(() => {
    finished.current = onFinished;
  }, [onFinished]);

  // A second import cannot start on top of a running one: there is a single
  // accumulator, and interleaving two channels would produce a tally that
  // belongs to neither. Dropping onto a busy window is ignored rather than
  // queued, because the user can simply drop again once the panel clears.
  const running = useRef(false);

  const start = useCallback(
    async (paths: string[]) => {
      if (running.current || paths.length === 0) return;
      running.current = true;
      setChoosing(false);
      setState({ ...IDLE_STATE, phase: "running" });

      try {
        await ingest.importFiles(paths, (event) =>
          setState((current) => reduceIngest(current, event)),
        );
      } catch (error) {
        toast(errorMessage(error), "error");
      } finally {
        running.current = false;
        // If the command rejected before its `finished` event, the panel would
        // otherwise sit at a progress bar that never completes.
        setState((current) =>
          current.phase === "running" ? { ...current, phase: "done" } : current,
        );
        finished.current();
      }
    },
    [toast],
  );

  /**
   * Native picker.
   *
   * `open`'s return type is computed from the literal `multiple`, which
   * widens away through generic inference - so the result is normalised by
   * hand rather than trusted.
   */
  const pick = useCallback(
    async (directory: boolean) => {
      try {
        const selected = (await open(
          directory
            ? { directory: true, multiple: true, title: "选择发票文件夹" }
            : {
                directory: false,
                multiple: true,
                filters: [INVOICE_FILTER],
                title: "选择发票文件",
              },
        )) as string | string[] | null;

        if (selected === null) return; // cancelled
        await start(Array.isArray(selected) ? selected : [selected]);
      } catch (error) {
        toast(errorMessage(error), "error");
      }
    },
    [start, toast],
  );

  /**
   * The shell has exactly one 导入发票 button, and a folder import needs a
   * different native dialog - `directory` is either/or on every platform's
   * open panel, there is no "files or folders" mode to fall back on. So the
   * button opens a two-line chooser instead of a dialog. It costs one click,
   * and buys the place to say which formats are understood *before* the
   * native dialog quietly greys out everything else.
   */
  const pickFiles = useCallback(() => {
    // Clears a finished summary first, so 继续导入 does not stack a chooser
    // on top of the previous import's results.
    setState((current) => (current.phase === "done" ? IDLE_STATE : current));
    setChoosing(true);
  }, []);
  const chooseFiles = useCallback(() => void pick(false), [pick]);
  const chooseFolder = useCallback(() => void pick(true), [pick]);

  const dismiss = useCallback(() => {
    setChoosing(false);
    setState((current) => (current.phase === "done" ? IDLE_STATE : current));
  }, []);

  // Window-wide, not per-pane: invoices arrive by mail while the user is
  // looking at last month's list, and every pane should accept them.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let stopped = false;

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "drop") {
          setDragging(false);
          void start(payload.paths);
        } else if (payload.type === "leave") {
          setDragging(false);
        } else {
          // "enter" and "over" - both mean the hint should be up. Treating
          // them alike also covers the platforms that only ever send one.
          setDragging(true);
        }
      })
      .then((fn) => {
        if (stopped) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // No webview (unit tests, a plain `vite dev` in a browser). Drag and
        // drop is simply unavailable there; the pickers still work.
      });

    return () => {
      stopped = true;
      unlisten?.();
    };
  }, [start]);

  return {
    ...state,
    dragging,
    choosing,
    pickFiles,
    chooseFiles,
    chooseFolder,
    dismiss,
  };
}
