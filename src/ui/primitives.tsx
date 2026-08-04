/**
 * The small pieces every pane is built from.
 *
 * Deliberately few and deliberately plain. A ledger's job is to make two
 * hundred rows scannable, and a component library with five button variants
 * and a shadow scale is how a table stops being scannable - so there is one
 * button with three intents, one badge, one modal, and nothing else.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { CloseIcon, SearchIcon } from "./icons";
import "./primitives.css";

// ---------------------------------------------------------------------------
// Button
// ---------------------------------------------------------------------------

type ButtonProps = {
  children: ReactNode;
  onClick?: () => void;
  /** `primary` for the one action a pane exists for; `danger` for anything
   *  that destroys data; `default` for everything else. */
  intent?: "default" | "primary" | "danger";
  disabled?: boolean;
  title?: string;
  /** Icon-only, square. */
  icon?: boolean;
  type?: "button" | "submit";
};

export function Button({
  children,
  onClick,
  intent = "default",
  disabled,
  title,
  icon,
  type = "button",
}: ButtonProps) {
  return (
    <button
      type={type}
      className={["btn", `btn-${intent}`, icon ? "btn-icon" : ""]
        .filter(Boolean)
        .join(" ")}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      {children}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Badge
// ---------------------------------------------------------------------------

export type BadgeTone = "neutral" | "ok" | "warn" | "danger" | "accent";

export function Badge({
  children,
  tone = "neutral",
  title,
}: {
  children: ReactNode;
  tone?: BadgeTone;
  title?: string;
}) {
  return (
    <span className={`badge badge-${tone}`} title={title}>
      {children}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

export function TextInput({
  value,
  onChange,
  placeholder,
  mono,
  invalid,
  disabled,
  onBlur,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  /** For amounts, invoice numbers and tax IDs. */
  mono?: boolean;
  invalid?: boolean;
  disabled?: boolean;
  onBlur?: () => void;
}) {
  return (
    <input
      className={["input", mono ? "tnum" : "", invalid ? "input-invalid" : ""]
        .filter(Boolean)
        .join(" ")}
      value={value}
      placeholder={placeholder}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value)}
      onBlur={onBlur}
    />
  );
}

export function SearchInput({
  value,
  onChange,
  placeholder = "搜索发票号码、销售方…",
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}) {
  return (
    <div className="search-field">
      <SearchIcon className="search-field-icon" />
      <input
        className="input search-field-input"
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
      {value && (
        <button
          className="search-field-clear"
          onClick={() => onChange("")}
          title="清除"
        >
          <CloseIcon />
        </button>
      )}
    </div>
  );
}

export function Select<T extends string>({
  value,
  onChange,
  options,
  disabled,
}: {
  value: T;
  onChange: (value: T) => void;
  options: { value: T; label: string }[];
  disabled?: boolean;
}) {
  return (
    <select
      className="input select"
      value={value}
      disabled={disabled}
      onChange={(event) => onChange(event.target.value as T)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

export function Toggle({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      className={`toggle ${checked ? "toggle-on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="toggle-knob" />
    </button>
  );
}

// ---------------------------------------------------------------------------
// Modal
// ---------------------------------------------------------------------------

export function Modal({
  title,
  onClose,
  children,
  footer,
  wide,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}) {
  // Escape closes. Registered on the document rather than the panel so it
  // works before anything inside has been focused.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div
        className={`modal ${wide ? "modal-wide" : ""}`}
        // Stops a drag that started inside the panel and ended on the
        // backdrop from closing it - which is what happens every time
        // someone selects text in a field and overshoots.
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="modal-header">
          <span>{title}</span>
          <button className="modal-close" onClick={onClose} title="关闭">
            <CloseIcon />
          </button>
        </header>
        <div className="modal-body">{children}</div>
        {footer && <footer className="modal-footer">{footer}</footer>}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Settings-style grouped rows, shared by the settings and rule panes
// ---------------------------------------------------------------------------

export function Group({
  title,
  children,
  hint,
}: {
  title: string;
  children: ReactNode;
  hint?: string;
}) {
  return (
    <section className="group">
      <h3 className="group-title">{title}</h3>
      <div className="group-body">{children}</div>
      {hint && <p className="group-hint">{hint}</p>}
    </section>
  );
}

export function Row({
  label,
  hint,
  children,
  stacked,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children?: ReactNode;
  /** Puts the control under the label instead of beside it, for wide inputs. */
  stacked?: boolean;
}) {
  return (
    <div className={`row ${stacked ? "row-stacked" : ""}`}>
      <div className="row-label">
        <span>{label}</span>
        {hint && <span className="row-hint">{hint}</span>}
      </div>
      {children && <div className="row-control">{children}</div>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

type Toast = { id: number; message: string; tone: "info" | "error" };

const ToastContext = createContext<
  (message: string, tone?: "info" | "error") => void
>(() => {});

/**
 * The app's only way of saying something went wrong.
 *
 * Every error the backend returns is already a Chinese sentence written for
 * the user (see the `Err(String)` payloads in `src-tauri/src/commands/`), so
 * the toast shows it verbatim rather than wrapping it in "操作失败：" - the
 * message already says what failed and usually what to do about it.
 */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(1);

  const push = useCallback(
    (message: string, tone: "info" | "error" = "info") => {
      const id = nextId.current++;
      setToasts((current) => [...current, { id, message, tone }]);
      // Errors linger: they usually name something the user has to go fix, and
      // four seconds is not enough to read a sentence and act on it.
      const lifetime = tone === "error" ? 8000 : 3000;
      setTimeout(
        () => setToasts((current) => current.filter((t) => t.id !== id)),
        lifetime,
      );
    },
    [],
  );

  return (
    <ToastContext.Provider value={push}>
      {children}
      <div className="toast-stack">
        {toasts.map((toast) => (
          <div key={toast.id} className={`toast toast-${toast.tone}`}>
            {toast.message}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  return useContext(ToastContext);
}

// ---------------------------------------------------------------------------
// Async data
// ---------------------------------------------------------------------------

/**
 * Runs `load` and re-runs it whenever `reload` is called.
 *
 * A stale-response guard is the point: filter changes fire faster than the
 * queries answer, and without the generation check a slow query for
 * "上个月" can land after a fast one for "本月" and repaint the table with
 * the wrong rows - with the wrong total under it.
 */
export function useAsync<T>(
  load: () => Promise<T>,
  deps: unknown[],
  initial: T,
): { data: T; loading: boolean; error: string | null; reload: () => void } {
  const [data, setData] = useState<T>(initial);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);
  const generation = useRef(0);

  // The latest-ref pattern, with the assignment in an effect rather than
  // during render: writing a ref while rendering is what `react-hooks/refs`
  // forbids, and it is genuinely unsafe under concurrent rendering, where a
  // render can be thrown away after the write. Declared BEFORE the loading
  // effect so that in any commit where `deps` changed, this one has already
  // run and `loadRef` holds the current closure.
  const loadRef = useRef(load);
  useEffect(() => {
    loadRef.current = load;
  });

  useEffect(() => {
    const current = ++generation.current;
    setLoading(true);
    loadRef
      .current()
      .then((result) => {
        if (current !== generation.current) return;
        setData(result);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (current !== generation.current) return;
        setError(typeof cause === "string" ? cause : String(cause));
      })
      .finally(() => {
        if (current === generation.current) setLoading(false);
      });
    // `load` is intentionally not a dependency - it is a fresh closure on
    // every render, and depending on it would loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce]);

  const reload = useCallback(() => setNonce((n) => n + 1), []);
  return useMemo(
    () => ({ data, loading, error, reload }),
    [data, loading, error, reload],
  );
}
