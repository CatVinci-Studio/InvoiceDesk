/**
 * A `YYYY-MM-DD` field.
 *
 * The one control this module adds to the primitives, and only because dates
 * are the one input where the native widget is strictly better: it enforces a
 * real calendar date, it opens a picker, and it localises the display without
 * changing the value - `input[type=date]` reads and writes exactly the
 * `YYYY-MM-DD` the whole app stores. Hand-rolling that in a `TextInput` would
 * mean re-implementing date validation in the toolbar and again in the
 * drawer.
 *
 * It borrows `.input` from the primitives rather than styling itself, so it
 * keeps the same height, radius and focus ring as everything beside it.
 */

export function DateInput({
  value,
  onChange,
  invalid,
  title,
  placeholder,
}: {
  /** `YYYY-MM-DD`, or `""` for unset. */
  value: string;
  onChange: (value: string) => void;
  invalid?: boolean;
  title?: string;
  /** Shown by the browser when the field is empty, where it supports it. */
  placeholder?: string;
}) {
  return (
    <input
      type="date"
      className={["input", "tnum", "date-input", invalid ? "input-invalid" : ""]
        .filter(Boolean)
        .join(" ")}
      value={value}
      title={title}
      placeholder={placeholder}
      // A date input hands back "" when it is cleared or half-typed, which is
      // exactly what "no bound" means to the filter and "unknown" means to
      // the drawer - so it passes straight through.
      onChange={(event) => onChange(event.target.value)}
    />
  );
}
