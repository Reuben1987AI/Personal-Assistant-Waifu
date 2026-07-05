import { useState, type CSSProperties, type ChangeEvent } from "react";

// Preemptive form-field scaffold (AGENTS.md → Form Validation). No forms exist
// yet; this is frozen so the first form (personality editor, context editor,
// appointments) uses the convention without reinventing it.
//
// - `required` + `maxLength` are enforced here (ephemeral focus/touched state
//   is owned by this component). Format/min errors come from the store via
//   `error`/`showError`. The widget shows "Required" on-blur when empty and
//   required, the store error otherwise.
// - The store's `submitBtnDisabled` still gates submit; this only displays.
export interface AppTextFieldProps {
  value: string;
  onChange: (v: string) => void;
  required?: boolean;
  maxLength?: number;
  error?: string | null;
  showError?: boolean;
  type?: "text" | "password" | "email" | "number";
  placeholder?: string;
  autoComplete?: string;
  style?: CSSProperties;
}

export function AppTextField({
  value,
  onChange,
  required,
  maxLength,
  error,
  showError,
  type = "text",
  placeholder,
  autoComplete,
  style,
}: AppTextFieldProps) {
  const [touched, setTouched] = useState(false);
  const empty = value.length === 0;
  const showRequired = required && empty && touched;
  const message = showRequired ? "Required" : showError && error ? error : null;

  function handleChange(e: ChangeEvent<HTMLInputElement>): void {
    const next = maxLength ? e.target.value.slice(0, maxLength) : e.target.value;
    onChange(next);
  }

  return (
    <label className="app-text-field" style={style}>
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        autoComplete={autoComplete}
        onChange={handleChange}
        onBlur={() => setTouched(true)}
      />
      {message && <span className="app-text-field-error">{message}</span>}
    </label>
  );
}
