/**
 * mode-toggle.tsx — Types / signatures mode control.
 *
 * Uses the shared `SegmentedControl` when the primitives are available, and a
 * native `<select>` otherwise.
 */

import React from "react";
import type { CodectMode } from "../../shared/schema";
import { SegmentedControl } from "../primitives";

export interface ModeToggleProps {
  value: CodectMode;
  onChange: (value: CodectMode) => void;
  label: string;
  types?: string;
  signatures?: string;
}

export function ModeToggle({
  value,
  onChange,
  label,
  types = "Types",
  signatures = "Signatures",
}: ModeToggleProps) {
  const options = [
    { value: "types" as CodectMode, label: types },
    { value: "signatures" as CodectMode, label: signatures },
  ];

  if (typeof SegmentedControl === "function") {
    return (
      <SegmentedControl
        className="codect-segmented"
        id="codect-mode"
        value={value}
        options={options}
        onChange={(next: CodectMode) => onChange(next)}
        label={label}
      />
    );
  }

  return (
    <select
      className="codect-input codect-segmented"
      value={value}
      aria-label={label}
      onChange={(e) => onChange(e.target.value as CodectMode)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}
