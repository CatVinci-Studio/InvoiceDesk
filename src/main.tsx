import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { restoreTheme } from "./settings/theme";
import { publishWindowChrome } from "./ui/window-chrome";

// Before the first render: the stylesheet keys the title row's left inset
// off this, and correcting it afterwards would paint the row at the macOS
// traffic-light inset for one frame on every Windows launch.
publishWindowChrome();

// Deliberately NOT awaited before rendering. The theme choice lives in the
// ledger, so reading it means opening SQLite - blocking the first paint on
// that would show an empty window on every launch. A user who chose 深色
// instead sees at most one frame in the system theme, which is the trade
// every app that stores its theme outside the DOM makes.
void restoreTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
