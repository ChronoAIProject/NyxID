import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
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
import { Check } from "lucide-react";
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
  useUiTranslation();
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
    <div className="flex flex-col gap-2.5">
      <Label>{translateUi(label)}</Label>
      <div
        role="group"
        aria-label={translateUi("Scopes")}
        aria-invalid={isEmpty}
        className={
          "flex flex-wrap gap-2 rounded-md transition-colors sm:grid sm:grid-cols-3 " +
          (isEmpty
            ? "ring-1 ring-destructive ring-offset-4 ring-offset-card"
            : "")
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
          isEmpty ? "text-13 text-destructive" : "text-13 text-muted-foreground"
        }
      >
        {isEmpty ? translateUi("At least one scope is required.") : hint}
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
  useUiTranslation();
  return (
    <Label
      className={
        "inline-flex cursor-pointer select-none items-center gap-1.5 rounded-md border px-2.5 py-2 transition-colors duration-300 " +
        (checked
          ? "border-primary/60 bg-primary/10 text-foreground"
          : "border-border/70 bg-transparent text-muted-foreground hover:border-border hover:bg-muted/40")
      }
    >
      <input
        type="checkbox"
        className="peer sr-only"
        checked={checked}
        onChange={onToggle}
        value={scope}
      />
      <Check aria-hidden="true" className={`size-3 shrink-0 text-primary-text ${checked ? "opacity-100" : "opacity-0"}`} />
      <span className="font-mono">{scope}</span>
    </Label>
  )
}
