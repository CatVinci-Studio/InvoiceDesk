/**
 * The appearance preference, and the one place that writes it to the DOM.
 *
 * `App.css` defines the dark palette twice on purpose: once under
 * `@media (prefers-color-scheme: dark) :root:not([data-theme="light"])` and
 * once under `:root[data-theme="dark"]`. That pair is what makes three
 * choices expressible with a single attribute:
 *
 * - `system` -> the attribute is emptied. It is then neither "light" nor
 *   "dark", so the media query decides and the explicit rule stays inert.
 * - `light`  -> `data-theme="light"` opts the media query out; nothing else
 *   matches, so the `:root` defaults (the light palette) stand.
 * - `dark`   -> `data-theme="dark"` matches the explicit rule regardless of
 *   what the OS reports.
 *
 * Emptying rather than removing the attribute is deliberate: `dataset.theme =
 * ""` leaves `data-theme=""` on the element, which satisfies
 * `:not([data-theme="light"])` exactly the same way an absent attribute
 * would, and keeps this function a single assignment with no branch that can
 * drift out of step with the stylesheet.
 */

import { prefs } from "../ipc";

export type ThemeChoice = "system" | "light" | "dark";

export const THEME_CHOICES: { value: ThemeChoice; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

/** The preference key. Namespaced to `pref.theme` by the Rust side. */
export const THEME_PREF_KEY = "theme";

export function applyTheme(choice: ThemeChoice): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = choice === "system" ? "" : choice;
}

function isThemeChoice(value: string | null): value is ThemeChoice {
  return value === "system" || value === "light" || value === "dark";
}

/**
 * Reads the stored choice, defaulting to `system`.
 *
 * Never rejects. A settings pane that refuses to open because the theme row
 * could not be read would be a poor trade, and "follow the OS" is the right
 * answer whenever we do not know better - including on the very first launch,
 * when nothing has been stored yet.
 */
export async function loadThemeChoice(): Promise<ThemeChoice> {
  try {
    const stored = await prefs.get(THEME_PREF_KEY);
    return isThemeChoice(stored) ? stored : "system";
  } catch {
    return "system";
  }
}

/**
 * Restores the stored theme. Exported for whoever owns app startup: today the
 * theme is applied when the settings pane first mounts, so a user who chose
 * 深色 sees the light palette until they visit 设置 again. Calling this once
 * before the first render closes that gap.
 */
export async function restoreTheme(): Promise<ThemeChoice> {
  const choice = await loadThemeChoice();
  applyTheme(choice);
  return choice;
}
