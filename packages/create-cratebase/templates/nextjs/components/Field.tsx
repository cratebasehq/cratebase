"use client";

import { useId, type InputHTMLAttributes } from "react";

interface FieldProps extends InputHTMLAttributes<HTMLInputElement> {
  label: string;
  /** Small helper text shown under the input and announced with it. */
  hint?: string;
}

/** A labelled text input — the label is always visible and wired to the input for screen readers. */
export function Field({ label, hint, ...props }: FieldProps) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-sm font-medium">
        {label}
      </label>
      <input id={id} className="input" aria-describedby={hint ? `${id}-hint` : undefined} {...props} />
      {hint && (
        <p id={`${id}-hint`} className="muted text-xs">
          {hint}
        </p>
      )}
    </div>
  );
}
