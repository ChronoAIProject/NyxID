// Upstream OAuth scope picker (NyxID#917 follow-up).
//
// Renders a provider's curated `scope_catalog` as selectable pills with the
// provider's `default_scopes` pre-selected (but removable — the user can drop
// a default), plus a free-form "add more" field for any scope not in the
// curated menu. Shared by the dashboard add-key dialog and the CLI pair
// wizard so both surfaces stay in lockstep.
//
// Controlled: the parent owns the selected-scope array and passes it to the
// connect request as `scope_override` (the complete set, replacing the
// additive default-merge). The parent seeds the initial value with the
// provider defaults so behavior is identical to before unless the user edits.

import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Plus, X } from "lucide-react";
import type { ScopeCatalogEntry } from "@/types/keys";
import { parseAdditionalScopes } from "@/lib/parse-additional-scopes";

export interface UpstreamScopePickerProps {
  /** Curated menu of selectable scopes for this provider (may be empty). */
  readonly catalog: readonly ScopeCatalogEntry[];
  /** Provider default scopes — shown and pre-selected, but removable. */
  readonly defaultScopes: readonly string[];
  /** Currently selected scopes (the complete set sent as scope_override). */
  readonly value: readonly string[];
  /** Called with the full updated selection on every change. */
  readonly onChange: (scopes: readonly string[]) => void;
  /** Free-form placeholder for the custom-scope input. */
  readonly customPlaceholder?: string;
  /** Optional id prefix so multiple pickers on one page keep unique ids. */
  readonly idPrefix?: string;
}

/** A pill row entry resolved from catalog ∪ defaults ∪ custom selections. */
interface PillEntry {
  readonly scope: string;
  readonly label: string;
  readonly description: string | null;
  readonly sensitive: boolean;
  /** True for the provider's default scopes (rendered with a subtle marker). */
  readonly isDefault: boolean;
}

/**
 * Build the ordered, deduped pill list: curated catalog entries first (in
 * their authored order), then any default scope not already in the catalog,
 * then any custom-added scope (present in `value` but unknown to both). This
 * guarantees defaults and custom additions always render as removable pills,
 * not just catalog scopes.
 */
function buildPills(
  catalog: readonly ScopeCatalogEntry[],
  defaultScopes: readonly string[],
  value: readonly string[],
): readonly PillEntry[] {
  const seen = new Set<string>();
  const defaults = new Set(defaultScopes);
  const pills: PillEntry[] = [];

  for (const e of catalog) {
    if (seen.has(e.scope)) continue;
    seen.add(e.scope);
    pills.push({
      scope: e.scope,
      label: e.label || e.scope,
      description: e.description || null,
      sensitive: Boolean(e.sensitive),
      isDefault: defaults.has(e.scope),
    });
  }
  for (const scope of defaultScopes) {
    if (seen.has(scope)) continue;
    seen.add(scope);
    pills.push({ scope, label: scope, description: null, sensitive: false, isDefault: true });
  }
  for (const scope of value) {
    if (seen.has(scope)) continue;
    seen.add(scope);
    // Custom scope the user typed — unknown to catalog and not a default.
    pills.push({ scope, label: scope, description: null, sensitive: false, isDefault: false });
  }
  return pills;
}

export function UpstreamScopePicker({
  catalog,
  defaultScopes,
  value,
  onChange,
  customPlaceholder = "e.g. custom.scope",
  idPrefix = "scope",
}: UpstreamScopePickerProps) {
  const [customInput, setCustomInput] = useState("");
  const selected = new Set(value);
  const pills = buildPills(catalog, defaultScopes, value);

  function toggle(scope: string) {
    const next = new Set(selected);
    if (next.has(scope)) {
      next.delete(scope);
    } else {
      next.add(scope);
    }
    // Preserve pill order in the emitted array for stable, readable output.
    onChange(pills.map((p) => p.scope).filter((s) => next.has(s)));
  }

  function addCustom() {
    const parsed = parseAdditionalScopes(customInput);
    if (parsed.length === 0) return;
    const next = [...value];
    for (const s of parsed) {
      if (!next.includes(s)) next.push(s);
    }
    setCustomInput("");
    onChange(next);
  }

  return (
    <div className="flex flex-col gap-2">
      <Label className="text-xs">Scopes</Label>
      {pills.length > 0 ? (
        <div role="group" aria-label="Scopes" className="flex flex-wrap gap-1.5">
          {pills.map((p) => {
            const isOn = selected.has(p.scope);
            return (
              <button
                key={p.scope}
                type="button"
                aria-pressed={isOn}
                title={p.description ?? p.scope}
                onClick={() => {
                  toggle(p.scope);
                }}
                className={
                  "group inline-flex max-w-full items-center gap-1.5 rounded-full border px-3 py-1.5 text-left text-[12px] transition-colors " +
                  (isOn
                    ? "border-primary bg-primary/15 text-foreground"
                    : "border-border bg-transparent text-muted-foreground hover:border-primary/50 hover:bg-muted/40")
                }
              >
                {p.sensitive ? (
                  <span
                    aria-hidden="true"
                    title="Write/admin-level scope"
                    className="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500"
                  />
                ) : null}
                <span className="truncate">{p.label}</span>
                {p.isDefault ? (
                  <span className="shrink-0 text-[10px] text-muted-foreground">
                    default
                  </span>
                ) : null}
                {isOn ? (
                  <X className="h-3 w-3 shrink-0 opacity-50 group-hover:opacity-100" />
                ) : null}
              </button>
            );
          })}
        </div>
      ) : null}

      <div className="flex items-center gap-1.5">
        <Input
          id={`${idPrefix}-custom`}
          value={customInput}
          onChange={(e) => {
            setCustomInput(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addCustom();
            }
          }}
          placeholder={customPlaceholder}
          autoComplete="off"
          spellCheck={false}
          className="h-9 text-[12px]"
        />
        <Button
          type="button"
          variant="outline"
          onClick={addCustom}
          disabled={customInput.trim().length === 0}
          className="h-9 shrink-0 px-3"
        >
          <Plus className="h-3.5 w-3.5" />
          Add
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {pills.length > 0
          ? "Selected scopes are requested at sign-in. Defaults are pre-selected — deselect to drop one. Add anything missing above; the upstream provider decides whether to grant them."
          : "Comma- or space-separated. The upstream provider decides whether to grant them."}
      </p>
    </div>
  );
}
