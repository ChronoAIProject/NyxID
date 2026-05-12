/**
 * Text input with inline Zod validation + optional helper hint.
 *
 * Used by every wizard confirm panel that accepts a free-text
 * identifier (node name, API key name, service slug, service label).
 *
 * Validates on every keystroke; surfaces the first issue from
 * `schema.safeParse(value).error.issues` as a red hint below the
 * field when the user has typed anything. Exposes the current
 * validation state via `onValidityChange` so the parent can disable
 * the submit button until the value parses clean.
 *
 * Source of truth: `frontend/src/schemas/cli-wizard.ts`.
 */

import { useEffect, useId } from "react"
import type { z } from "zod"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { firstError } from "@/schemas/cli-wizard"

export interface NameInputProps {
  readonly label: string
  /** Zod schema to validate against. */
  readonly schema: z.ZodType<string>
  /** Current input value (controlled). */
  readonly value: string
  /** Called on every keystroke. */
  readonly onChange: (next: string) => void
  /** Called whenever validity flips so parent can disable submit. */
  readonly onValidityChange?: (valid: boolean) => void
  /** Optional helper text rendered below the input when valid. */
  readonly hint?: string
  /** Optional placeholder for the input. */
  readonly placeholder?: string
  /** Whether the field is optional. Empty is treated as valid. */
  readonly optional?: boolean
  /** Extra HTML attributes for accessibility / autocomplete tuning. */
  readonly autoFocus?: boolean
  readonly autoComplete?: string
  readonly id?: string
}

export function NameInput({
  label,
  schema,
  value,
  onChange,
  onValidityChange,
  hint,
  placeholder,
  optional = false,
  autoFocus,
  autoComplete = "off",
  id: idProp,
}: NameInputProps) {
  const reactId = useId()
  const id = idProp ?? reactId

  // Optional + empty → valid and no error message.
  const effectiveError =
    optional && value.length === 0 ? null : firstError(schema, value)

  // Bubble validity up so the parent can disable the submit button.
  useEffect(() => {
    if (onValidityChange) {
      onValidityChange(effectiveError === null)
    }
  }, [effectiveError, onValidityChange])

  const hintId = `${id}-hint`
  const errorId = `${id}-error`

  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        value={value}
        onChange={(e) => {
          onChange(e.target.value)
        }}
        placeholder={placeholder}
        autoFocus={autoFocus}
        autoComplete={autoComplete}
        aria-invalid={effectiveError != null}
        aria-describedby={effectiveError ? errorId : hint ? hintId : undefined}
        className={
          effectiveError != null
            ? "border-destructive focus-visible:border-destructive"
            : undefined
        }
      />
      {effectiveError ? (
        <p id={errorId} className="text-xs text-destructive">
          {effectiveError}
        </p>
      ) : hint ? (
        <p id={hintId} className="text-xs text-muted-foreground">
          {hint}
        </p>
      ) : null}
    </div>
  )
}
