import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStatus =
  "idle" | "checking" | "available" | "downloading" | "error";

/**
 * Checks GitHub Releases for a newer build and drives the update banner:
 * available → user confirms → download and install → relaunch.
 *
 * Adapted from the hook of the same name in CatVinci's Levis. The endpoint is
 * the latest release's `latest.json` (see `plugins.updater` in
 * tauri.conf.json), which GitHub only points at non-prerelease tags - so an
 * `-rc` build never reaches anyone on a stable install.
 *
 * ## Two failure policies, on purpose
 *
 * The **automatic** check at startup fails silently. Being offline, being
 * rate-limited, or running under `vite dev` with no Tauri host are all normal
 * situations, and an app that opens with "更新检查失败" on a train is worse
 * than one that quietly tries again later.
 *
 * A **manual** check does the opposite: the user pressed a button and is
 * owed an answer, including "已经是最新版本" and including the error. That is
 * what `checkNow` is for, and why it returns a result the caller can report
 * rather than only setting state.
 */
export function useAppUpdate() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const checkOnce = () => {
      check()
        .then((found) => {
          if (!cancelled && found) {
            setUpdate(found);
            setStatus("available");
          }
        })
        .catch(() => {
          // Silent by design - see the hook comment.
        });
    };

    // Once at startup, then every few hours: this app gets left open for a
    // whole afternoon of expense sorting, and a window that never re-checks
    // would never hear about a release published while it sat there.
    checkOnce();
    const timer = setInterval(checkOnce, 4 * 60 * 60 * 1000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  /** A user-initiated check. Resolves to what to tell them. */
  const checkNow = useCallback(async (): Promise<string> => {
    setStatus("checking");
    try {
      const found = await check();
      if (found) {
        setUpdate(found);
        setStatus("available");
        return `发现新版本 ${found.version}`;
      }
      setStatus("idle");
      return "已经是最新版本";
    } catch (cause) {
      setStatus("idle");
      // Not stored in `error` - that field drives the banner, and a failed
      // check is not something to leave on screen.
      return `检查更新失败：${cause instanceof Error ? cause.message : String(cause)}`;
    }
  }, []);

  const install = useCallback(async () => {
    if (!update) return;
    setStatus("downloading");
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setStatus("error");
    }
  }, [update]);

  const dismiss = useCallback(() => {
    setUpdate(null);
    setStatus("idle");
    setError(null);
  }, []);

  return {
    version: update?.version ?? null,
    notes: update?.body ?? null,
    status,
    error,
    checkNow,
    install,
    dismiss,
  };
}
