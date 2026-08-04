/**
 * Everything an import puts on screen: the drop hint, the chooser, the
 * progress panel, and the summary that stays until it is dismissed.
 *
 * The summary is the reason this file is longer than the hook. An import is
 * over in seconds and nobody watches the bar; what people actually need is
 * the list afterwards - which files still want a human, and why. So the
 * cleanly imported ones collapse into a single number and everything else
 * gets a row, a reason, and the trace of which extraction layer was tried.
 */

import { useState } from "react";
import { FILE_STATUS_LABEL, type FileStatus, type TraceStep } from "../types";
import {
  Badge,
  Button,
  Group,
  Modal,
  Row,
  useToast,
  type BadgeTone,
} from "../ui/primitives";
import { CheckIcon, ChevronIcon, ImportIcon, SparkIcon } from "../ui/icons";
import {
  SUPPORTED_FORMATS,
  type FileResult,
  type Importer,
  type Tally,
} from "./useImport";
import "./ingest.css";

// ---------------------------------------------------------------------------
// Status vocabulary
// ---------------------------------------------------------------------------

/**
 * 重复 is deliberately not red.
 *
 * `alreadyImported` means the user dropped the same file twice - which is
 * what happens every time someone re-imports a folder after adding two new
 * invoices to it. Nothing went wrong, nothing was lost, and nothing needs
 * doing; colouring it like a failure would train people to ignore the colour
 * that does mean something.
 */
const STATUS_TONE: Record<FileStatus, BadgeTone> = {
  imported: "ok",
  needsReview: "warn",
  alreadyImported: "neutral",
  failed: "danger",
};

/**
 * The tally is four counts because a file can end in exactly four states, and
 * each implies a different next action: nothing, 去复核, ignore it, 手动补录.
 * Collapsing them (say, into "成功 / 失败") would hide the only two that ask
 * anything of the user.
 */
const TALLY_CELLS: { key: FileStatus; label: string; tone: string }[] = [
  { key: "imported", label: "已导入", tone: "ok" },
  { key: "needsReview", label: "待复核", tone: "warn" },
  { key: "alreadyImported", label: "重复", tone: "muted" },
  { key: "failed", label: "失败", tone: "danger" },
];

function TallyStrip({ tally }: { tally: Tally }) {
  return (
    <div className="ingest-tally">
      {TALLY_CELLS.map((cell) => (
        <div key={cell.key} className="ingest-tally-cell">
          <span className={`ingest-tally-count tnum ingest-tone-${cell.tone}`}>
            {tally[cell.key]}
          </span>
          <span className="ingest-tally-label">{cell.label}</span>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Drop hint
// ---------------------------------------------------------------------------

/**
 * Shown for as long as a drag is over the window.
 *
 * `pointer-events: none` throughout: the drag is handled by the OS through
 * Tauri, and an overlay that took the pointer would only give the webview a
 * chance to swallow the drop.
 */
function DropHint() {
  return (
    <div className="ingest-drop">
      <div className="ingest-drop-frame">
        <ImportIcon className="ingest-drop-icon" />
        <div className="ingest-drop-title">把发票文件或文件夹拖进来</div>
        <div className="ingest-drop-formats">支持 {SUPPORTED_FORMATS}</div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Chooser
// ---------------------------------------------------------------------------

/** The one place that says what is understood *before* a native dialog greys
 *  half the folder out with no explanation. */
function Chooser({ importer }: { importer: Importer }) {
  return (
    <Modal title="导入发票" onClose={importer.dismiss}>
      <Group
        title="从哪里导入"
        hint={`支持 ${SUPPORTED_FORMATS}。也可以直接把文件或文件夹拖进窗口，任何界面都收。`}
      >
        <Row label="选择文件" hint="可以一次选中多张发票">
          <Button intent="primary" onClick={importer.chooseFiles}>
            选择文件…
          </Button>
        </Row>
        <Row label="选择文件夹" hint="连同子文件夹一起读，压缩包会自动解开">
          <Button onClick={importer.chooseFolder}>选择文件夹…</Button>
        </Row>
      </Group>
    </Modal>
  );
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

/** No close button on purpose: the import is writing to the ledger, and a
 *  panel that can be dismissed mid-write reads as if dismissing cancels it. */
function Progress({ importer }: { importer: Importer }) {
  const { done, total, currentName, tally } = importer;
  const percent = total > 0 ? Math.round((done / total) * 100) : 0;

  return (
    <div className="ingest-backdrop">
      <div className="ingest-panel">
        <div className="ingest-panel-head">
          <span className="ingest-panel-title">正在导入</span>
          <span className="ingest-panel-count tnum">
            {done} / {total}
          </span>
        </div>

        <div
          className="ingest-bar"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={total}
          aria-valuenow={done}
        >
          <div className="ingest-bar-fill" style={{ width: `${percent}%` }} />
        </div>

        <div className="ingest-current" title={currentName ?? undefined}>
          {/* The backend expands folders and unpacks zips before the first
              file event, which on a big folder is a visible pause. */}
          {currentName ?? "正在展开文件夹…"}
        </div>

        <TallyStrip tally={tally} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// The vision suggestion
// ---------------------------------------------------------------------------

/**
 * Shown once per import, never per file.
 *
 * Photos and scans have no text layer and often no readable QR code, so the
 * offline layers genuinely cannot finish them - and the user has no way to
 * know that a switch exists which would. But it is a suggestion, not a
 * defect: it is phrased in the same quiet grey as everything else, it names
 * the cost (the images leave the machine) in the same breath as the benefit,
 * and it appears once no matter how many photos were in the folder. Repeating
 * it per row would turn a helpful note into nagging, and nagging is how a
 * privacy trade-off gets clicked through without being read.
 */
function VisionNote({
  count,
  onNavigated,
}: {
  count: number;
  onNavigated: () => void;
}) {
  const toast = useToast();

  const openSettings = () => {
    // Navigation belongs to the shell and the overlay has no handle on it, so
    // this asks rather than reaches: a shell that wants to route the user
    // claims the event with `preventDefault()`. If nobody does, saying where
    // the setting lives beats a button that visibly does nothing.
    const request = new CustomEvent("zhishui:open-settings", {
      detail: { section: "ai" },
      cancelable: true,
    });
    window.dispatchEvent(request);

    if (request.defaultPrevented) onNavigated();
    else toast("在左边的「设置」里打开「AI 识别」。");
  };

  return (
    <div className="ingest-note">
      <SparkIcon className="ingest-note-icon" />
      <div className="ingest-note-body">
        <p className="ingest-note-text">
          其中 <span className="tnum">{count}</span>{" "}
          个是照片或扫描件，离线的几层（PDF 文本层、二维码）读不出字来，只能靠
          AI 识别。这项现在是关着的 —— 开启后图片会上传到所选服务商，由它来读。
        </p>
        <Button onClick={openSettings}>去设置里开启</Button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

function TraceList({ trace }: { trace: TraceStep[] }) {
  if (trace.length === 0) {
    return <div className="ingest-trace-empty">没有记录识别过程。</div>;
  }
  return (
    <ol className="ingest-trace">
      {trace.map((step, index) => (
        <li
          key={`${step.layer}-${index}`}
          className={`ingest-step ${step.hit ? "ingest-step-hit" : ""}`}
        >
          <span className="ingest-step-mark">
            {step.hit ? <CheckIcon /> : "—"}
          </span>
          <span className="ingest-step-layer">{step.layer}</span>
          <span className="ingest-step-detail">{step.detail}</span>
        </li>
      ))}
    </ol>
  );
}

/** `alreadyImported` carries no message from the backend - there is no
 *  problem to describe - so it gets one that says as much. */
function messageFor(result: FileResult): string {
  if (result.message) return result.message;
  if (result.status === "alreadyImported") {
    return "和账本里已有的一张完全相同，没有重复入账。";
  }
  return "";
}

function ProblemRow({ result }: { result: FileResult }) {
  const [open, setOpen] = useState(false);
  const message = messageFor(result);

  return (
    <div className="ingest-problem">
      <button
        className="ingest-problem-head"
        onClick={() => setOpen((current) => !current)}
        aria-expanded={open}
        title="识别过程"
      >
        <ChevronIcon
          className={`ingest-chevron ${open ? "ingest-chevron-open" : ""}`}
        />
        <span className="ingest-problem-name">{result.name}</span>
        <Badge tone={STATUS_TONE[result.status]}>
          {FILE_STATUS_LABEL[result.status]}
        </Badge>
      </button>

      {message && <p className="ingest-problem-message">{message}</p>}
      {open && <TraceList trace={result.trace} />}
    </div>
  );
}

function Summary({ importer }: { importer: Importer }) {
  const { tally, total, problems, visionCandidates, dismiss } = importer;

  return (
    <Modal
      title="导入完成"
      onClose={dismiss}
      footer={
        <>
          <Button onClick={importer.pickFiles}>继续导入</Button>
          <Button intent="primary" onClick={dismiss}>
            知道了
          </Button>
        </>
      }
    >
      <TallyStrip tally={tally} />

      {total === 0 && (
        <p className="ingest-summary-note">这些路径里没有找到可以读的文件。</p>
      )}

      {visionCandidates > 0 && (
        <VisionNote count={visionCandidates} onNavigated={dismiss} />
      )}

      {tally.failed > 0 && (
        // Said once, above the list: the fear on seeing 识别失败 is that the
        // file was thrown away. It was not - `db::save` runs either way.
        <p className="ingest-summary-note">
          识别失败的文件也已经存进账本了，字段可以之后自己补录，不用重新找文件。
        </p>
      )}

      {problems.length > 0 ? (
        <div className="ingest-problems">
          <div className="ingest-problems-title">
            需要你看一眼的（<span className="tnum">{problems.length}</span>）
          </div>
          {problems.map((result) => (
            <ProblemRow key={result.index} result={result} />
          ))}
        </div>
      ) : (
        total > 0 && (
          <p className="ingest-summary-note">全部读干净了，没有需要处理的。</p>
        )
      )}
    </Modal>
  );
}

// ---------------------------------------------------------------------------

export function ImportOverlay({ importer }: { importer: Importer }) {
  return (
    <>
      {importer.choosing && <Chooser importer={importer} />}
      {importer.phase === "running" && <Progress importer={importer} />}
      {importer.phase === "done" && <Summary importer={importer} />}
      {/* Last, so the hint sits over a summary that is still open - dropping
          onto it is a perfectly reasonable way to start the next import. */}
      {importer.dragging && <DropHint />}
    </>
  );
}
