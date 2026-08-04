import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  WindowCloseIcon,
  WindowMaximizeIcon,
  WindowMinimizeIcon,
  WindowRestoreIcon,
} from "./icons";
import { appDrawsWindowFrame } from "./window-chrome";
import "./window-controls.css";

/**
 * The caption buttons, drawn by the app where the OS no longer draws them.
 *
 * Renders null on macOS, where the traffic lights float over our title row
 * unchanged - so the call site places this unconditionally and the macOS
 * layout stays byte-for-byte what it was. See window-chrome.ts for why the
 * two platforms differ.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!appDrawsWindowFrame) return;
    const window = getCurrentWindow();
    let cancelled = false;

    void window.isMaximized().then((value) => {
      if (!cancelled) setMaximized(value);
    });

    // The maximize glyph has to track the window even when the user got
    // there another way - double-clicking the title row, Win+Up, or snapping
    // to an edge - so it follows the resize event rather than only the click.
    const unlisten = window.onResized(() => {
      void window.isMaximized().then((value) => {
        if (!cancelled) setMaximized(value);
      });
    });

    return () => {
      cancelled = true;
      void unlisten.then((off) => off());
    };
  }, []);

  if (!appDrawsWindowFrame) return null;
  const window = getCurrentWindow();

  return (
    <div className="window-caption">
      <button
        className="window-caption-button"
        onClick={() => void window.minimize()}
        title="最小化"
      >
        <WindowMinimizeIcon />
      </button>
      <button
        className="window-caption-button"
        onClick={() => void window.toggleMaximize()}
        title={maximized ? "还原" : "最大化"}
      >
        {maximized ? <WindowRestoreIcon /> : <WindowMaximizeIcon />}
      </button>
      <button
        className="window-caption-button window-caption-close"
        onClick={() => void window.close()}
        title="关闭"
      >
        <WindowCloseIcon />
      </button>
    </div>
  );
}
