/**
 * Scope chip row — 9 pill-style checkboxes for the NyxID API-key scope
 * set. Ported from Mode A's `.wizard-scope-chip-row` in
 * `cli/src/wizard/assets/wizard.css:506-546` and populated from
 * `wizard.js:1958-1968` (`SCOPE_OPTIONS`).
 *
 * Source of truth for the list: `API_KEY_SCOPES` in
 * `frontend/src/schemas/api-keys.ts`, which mirrors
 * `backend/src/services/key_service.rs::VALID_API_KEY_SCOPES`.
 *
 * Visual: an unchecked chip renders as a subtle outline; the `:checked`
 * sibling `<span>` fills in with the primary/accent background — same
 * mechanic as Mode A's `:has(:checked)` selector, implemented here with
 * Tailwind `peer` + `peer-checked` utilities to keep the JSX readable.
 */

import { API_KEY_SCOPES, type ApiKeyScope } from "@/schemas/api-keys"
import { Label } from "@/components/ui/label"

export interface ScopePickerProps {
  /** Currently selected scopes. */
  readonly value: ReadonlySet<ApiKeyScope>
  /** Called with the full updated set on every toggle. */
  readonly onChange: (next: Set<ApiKeyScope>) => void
  /** Optional label rendered above the chip row. */
  readonly label?: string
  /** Optional helper text. */
  readonly hint?: string
}

export function ScopePicker({
  value,
  onChange,
  label = "Scopes",
  hint = "Must match the backend's allowed scope set. Pick at least one.",
}: ScopePickerProps) {
  function toggle(scope: ApiKeyScope) {
    const next = new Set(value)
    if (next.has(scope)) {
      next.delete(scope)
    } else {
      next.add(scope)
    }
    onChange(next)
  }

  const isEmpty = value.size === 0

  return (
    <div className="flex flex-col gap-1.5">
      <Label>{label}</Label>
      <div
        role="group"
        aria-label="Scopes"
        aria-invalid={isEmpty}
        className={
          "flex flex-wrap gap-2 rounded-lg p-2 transition-colors duration-300 " +
          (isEmpty
            ? "border border-destructive"
            : "border border-transparent")
        }
      >
        {API_KEY_SCOPES.map((scope) => (
          <ScopeChip
            key={scope}
            scope={scope}
            checked={value.has(scope)}
            onToggle={() => {
              toggle(scope)
            }}
          />
        ))}
      </div>
      <p
        className={
          isEmpty ? "text-xs text-destructive" : "text-xs text-muted-foreground"
        }
      >
        {isEmpty ? "At least one scope is required." : hint}
      </p>
    </div>
  )
}

function ScopeChip({
  scope,
  checked,
  onToggle,
}: {
  readonly scope: ApiKeyScope
  readonly checked: boolean
  readonly onToggle: () => void
}) {
  return (
    <label
      className={
        "inline-flex cursor-pointer select-none items-center gap-1.5 rounded-full border px-3 py-1.5 text-[12px] transition-colors duration-300 " +
        (checked
          ? "border-primary bg-primary/15 text-foreground"
          : "border-border bg-transparent text-muted-foreground hover:border-border hover:bg-muted/40")
      }
    >
      <input
        type="checkbox"
        className="peer sr-only"
        checked={checked}
        onChange={onToggle}
        value={scope}
      />
      <span className="text-xs">{scope}</span>
    </label>
  )
}
