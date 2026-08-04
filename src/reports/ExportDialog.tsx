/**
 * The last step: writing the sheet out.
 *
 * Two paths, because Chinese companies split cleanly into two camps (see the
 * module doc on `src-tauri/src/report/mod.rs`):
 *
 *   通用 Excel  - a clean workbook we generate, 明细表 + 分类汇总表
 *   公司模板    - the company's OWN .xlsx, filled in place, formatting intact
 *
 * Both are offered at once rather than behind a mode switch: which one a user
 * needs is fixed by their employer, they will use the same one every month,
 * and the second one has to explain itself the first time it is seen. So the
 * placeholder reference lives right here, folded away, next to the button
 * that needs it.
 */

import { useCallback, useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { countLabel } from "../format";
import { errorMessage, prefs, reports as reportsApi } from "../ipc";
import type { FillReport, ReportMeta } from "../types";
import { ExternalIcon } from "../ui/icons";
import {
  Button,
  Group,
  Modal,
  Row,
  useAsync,
  useToast,
} from "../ui/primitives";
import { baseName, safeFileName } from "./rows";

/** Where the company template path is remembered between months. */
const TEMPLATE_PREF = "templatePath";

const XLSX_FILTER = [{ name: "Excel 工作簿", extensions: ["xlsx"] }];

export function ExportDialog({
  reportId,
  meta,
  invoiceCount,
  duplicateCount,
  onClose,
}: {
  reportId: number;
  meta: ReportMeta;
  invoiceCount: number;
  /** Repeated here on purpose - see the note on the banner below. */
  duplicateCount: number;
  onClose: () => void;
}) {
  const toast = useToast();
  const [templatePath, setTemplatePath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{
    path: string;
    fill: FillReport | null;
  } | null>(null);

  useEffect(() => {
    let alive = true;
    prefs
      .get(TEMPLATE_PREF)
      .then((stored) => {
        if (alive) setTemplatePath(stored);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const { data: placeholders } = useAsync<[string, string][]>(
    () => reportsApi.templatePlaceholders(),
    [],
    [],
  );

  /** Asks for the output path. `null` means the user cancelled. */
  const askWhereToSave = useCallback(async () => {
    return save({
      defaultPath: `${safeFileName(meta.title)}.xlsx`,
      filters: XLSX_FILTER,
    });
  }, [meta.title]);

  const exportGeneric = useCallback(async () => {
    try {
      const outPath = await askWhereToSave();
      if (!outPath) return;
      setBusy(true);
      await reportsApi.exportXlsx(reportId, meta, outPath);
      setResult({ path: outPath, fill: null });
    } catch (error) {
      toast(errorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  }, [askWhereToSave, meta, reportId, toast]);

  const chooseTemplate = useCallback(async () => {
    try {
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Excel 模板", extensions: ["xlsx", "xlsm"] }],
      });
      if (typeof picked !== "string") return;
      setTemplatePath(picked);
      // Remembered so next month's export is two clicks. A mandated form
      // changes once a year at most; being asked for it every time is the
      // kind of friction that sends people back to doing this by hand.
      await prefs.set(TEMPLATE_PREF, picked);
    } catch (error) {
      toast(errorMessage(error), "error");
    }
  }, [toast]);

  const exportWithTemplate = useCallback(async () => {
    if (!templatePath) return;
    try {
      const outPath = await askWhereToSave();
      if (!outPath) return;
      setBusy(true);
      const fill = await reportsApi.exportWithTemplate(
        reportId,
        meta,
        templatePath,
        outPath,
      );
      setResult({ path: outPath, fill });
    } catch (error) {
      toast(errorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  }, [askWhereToSave, meta, reportId, templatePath, toast]);

  const reveal = useCallback(
    async (path: string) => {
      try {
        await openPath(path);
      } catch (error) {
        toast(errorMessage(error), "error");
      }
    },
    [toast],
  );

  const scalarPlaceholders = placeholders.filter(
    ([token]) => !token.startsWith("{{明细."),
  );
  const detailPlaceholders = placeholders.filter(([token]) =>
    token.startsWith("{{明细."),
  );

  return (
    <Modal
      title="导出报销单"
      onClose={onClose}
      wide
      footer={<Button onClick={onClose}>关闭</Button>}
    >
      {/* The duplicate count is shown in the builder already; it is repeated
          here because this dialog is the last surface before a file exists,
          and a warning that was scrolled past ten minutes ago is a warning
          nobody read. */}
      {duplicateCount > 0 && (
        <div className="rp-alert rp-alert-danger rp-alert-flush">
          <div className="rp-alert-title">
            这张单上有 {duplicateCount} 张疑似重复的发票
          </div>
          <p className="rp-alert-body">
            它们在发票库里还有同号的另一张，可能已经报销过。导出的文件一旦交出去就很难撤回，请先确认。
          </p>
        </div>
      )}

      {result && (
        <div className="rp-result">
          <div className="rp-result-head">
            <span className="rp-result-title">已导出</span>
            <Button onClick={() => void reveal(result.path)}>
              <ExternalIcon />
              打开文件
            </Button>
          </div>
          <p className="rp-result-path">{result.path}</p>
          {result.fill && (
            <>
              <p className="rp-result-note">
                模板里用到了 {result.fill.filled.length} 个占位符，写入明细{" "}
                {countLabel(result.fill.detailRows)}。
              </p>
              {result.fill.missing.length > 0 && (
                <div className="rp-alert rp-alert-warn">
                  {/* Not an error - a form without a 可抵扣税额 column simply
                      does not want one. But a column the user believed was
                      there and silently was not is how a wrong sheet reaches
                      finance, so the list is shown rather than swallowed. */}
                  <div className="rp-alert-title">
                    模板里没有用到的占位符（{result.fill.missing.length} 个）
                  </div>
                  <p className="rp-alert-body">
                    这些内容没有写进文件，因为模板里找不到对应的占位符。本来就不需要这些列的话，可以忽略；
                    如果是漏写了，在模板里补上对应占位符再导出一次。
                  </p>
                  <div className="rp-chips">
                    {result.fill.missing.map((token) => (
                      <span key={token} className="rp-chip tnum">
                        {token}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      )}

      <Group
        title="通用 Excel"
        hint="生成一个新的工作簿：「报销明细」一张发票一行，含金额、税额、可抵扣与校验问题；「分类汇总」按费用类别合计并给出占比。表尾有合计与人民币大写。"
      >
        <Row label="导出为标准报销单" hint={`当前 ${countLabel(invoiceCount)}`}>
          <Button
            intent="primary"
            disabled={busy}
            onClick={() => void exportGeneric()}
          >
            导出…
          </Button>
        </Row>
      </Group>

      <Group
        title="公司模板"
        hint="用公司规定的报销单模板导出，字体、边框、公章位置和签字栏都原样保留，只把占位符换成数据。"
      >
        <Row
          label="模板文件"
          hint={templatePath ? templatePath : "还没有选择模板"}
        >
          <Button onClick={() => void chooseTemplate()}>
            {templatePath ? baseName(templatePath) : "选择模板…"}
          </Button>
        </Row>
        <Row label="导出为公司模板" hint="会记住这个模板，下个月不用再选">
          <Button
            intent="primary"
            disabled={busy || !templatePath}
            onClick={() => void exportWithTemplate()}
          >
            导出…
          </Button>
        </Row>
      </Group>

      {/* Folded away by default: the reference is essential the first time
          and noise every month after. */}
      <details className="rp-details">
        <summary className="rp-details-summary">模板里能写哪些占位符？</summary>
        <div className="rp-details-body">
          <p>
            把下面这些占位符直接打进公司模板的单元格里，位置随意，可以和文字写在一起
            （例如「申请人：{"{{申请人}}"}
            」）。导出时只替换占位符本身，模板的其余内容一个字节都不动。
          </p>
          <p>
            <strong>含 {"{{明细.*}}"} 的那一行是明细模板行</strong>
            ：它会按报销单里的发票复制若干份，每份都带着这一行原本的格式，下面的内容自动往下移。
            所以模板里只写一行明细就够了。报销单是空的时候，这一行会被删掉，不会把{" "}
            {"{{明细.日期}}"} 留在交出去的表里。
          </p>
          <p className="rp-details-note">
            金额类占位符写进去的是数字而不是文本，Excel 里可以直接 SUM。
          </p>

          <PlaceholderTable caption="表头与汇总" rows={scalarPlaceholders} />
          <PlaceholderTable
            caption="明细行（每张发票一行）"
            rows={detailPlaceholders}
          />
        </div>
      </details>
    </Modal>
  );
}

function PlaceholderTable({
  caption,
  rows,
}: {
  caption: string;
  rows: [string, string][];
}) {
  if (rows.length === 0) return null;
  return (
    <table className="rp-table rp-ph-table">
      <thead>
        <tr>
          <th colSpan={2} className="rp-ph-caption">
            {caption}
          </th>
        </tr>
        <tr>
          <th>占位符</th>
          <th>含义</th>
        </tr>
      </thead>
      <tbody>
        {rows.map(([token, meaning]) => (
          <tr key={token}>
            <td className="tnum rp-ph-token">{token}</td>
            <td>{meaning}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
