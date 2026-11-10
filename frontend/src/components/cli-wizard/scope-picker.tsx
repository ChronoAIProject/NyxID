import { useWizardTranslation } from "@/i18n/wizard";
/** Scope values mirror the backend's VALID_API_KEY_SCOPES allowlist. */

import { API_KEY_SCOPES, type ApiKeyScope } from "@/schemas/api-keys"
import { Label } from "@/components/ui/label"
import { Check } from "lucide-react"

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
  const { t } = useWizardTranslation();
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
      <Label className="text-12 text-foreground">{t(label)}</Label>
      <div
        role="group"
        aria-label={t("Scopes")}
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
          isEmpty ? "text-xs text-destructive" : "text-xs text-muted-foreground"
        }
      >
        {isEmpty ? t("At least one scope is required.") : t(hint)}
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
        "inline-flex cursor-pointer select-none items-center gap-1.5 rounded-md border px-2.5 py-2 text-12 transition-colors focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 focus-within:ring-offset-card " +
        (checked
          ? "border-primary/60 bg-primary/10 text-foreground"
          : "border-border/70 bg-transparent text-muted-foreground hover:border-border hover:bg-overlay")
      }
    >
      <input
        type="checkbox"
        className="sr-only"
        checked={checked}
        onChange={onToggle}
        value={scope}
      />
      <Check
        aria-hidden="true"
        className={`size-3 shrink-0 text-primary-text ${checked ? "opacity-100" : "opacity-0"}`}
      />
      <span className="font-mono text-xs">{scope}</span>
    </label>
  )
}
