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
  /**
   * Scopes already granted on an existing connection (append-only edit,
   * NyxID#917 follow-up). Rendered selected + non-removable (a "granted" tag,
   * no ×) and always kept in the emitted set — the user can add scopes but not
   * drop a current one (removal/revoke is a later, separate flow). Empty/unset
   * = fresh add, where defaults are removable.
   */
  readonly lockedScopes?: readonly string[];
  /**
   * The connection's currently-granted scopes when editing an existing
   * connection (NyxID#917 follow-up). Drives the change summary + removal
   * warning. Distinct from `lockedScopes`: `grantedScopes` is what they have
   * now (for the diff); `lockedScopes` is the subset that can't be removed
   * (used when the provider's `scope_removal` is `unsupported`). Unset = a
   * fresh add (no diff shown).
   */
  readonly grantedScopes?: readonly string[];
  /**
   * Human provider name for the removal warning copy (e.g. "Twitter / X").
   * Falls back to "the provider".
   */
  readonly providerName?: string;
}

/** A pill row entry resolved from catalog ∪ defaults ∪ locked ∪ custom. */
interface PillEntry {
  readonly scope: string;
  readonly label: string;
  readonly description: string | null;
  readonly sensitive: boolean;
  /** True for the provider's default scopes (rendered with a subtle marker). */
  readonly isDefault: boolean;
  /** True for already-granted scopes that can't be removed in edit mode. */
  readonly locked: boolean;
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
  lockedScopes: readonly string[],
): readonly PillEntry[] {
  const seen = new Set<string>();
  const defaults = new Set(defaultScopes);
  const locked = new Set(lockedScopes);
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
      locked: locked.has(e.scope),
    });
  }
  // Locked (already-granted) scopes that aren't in the catalog must still
  // render — they're part of the connection's grant.
  for (const scope of lockedScopes) {
    if (seen.has(scope)) continue;
    seen.add(scope);
    pills.push({ scope, label: scope, description: null, sensitive: false, isDefault: false, locked: true });
  }
  for (const scope of defaultScopes) {
    if (seen.has(scope)) continue;
    seen.add(scope);
    pills.push({ scope, label: scope, description: null, sensitive: false, isDefault: true, locked: false });
  }
  for (const scope of value) {
    if (seen.has(scope)) continue;
    seen.add(scope);
    // Custom scope the user typed — unknown to catalog and not a default.
    pills.push({ scope, label: scope, description: null, sensitive: false, isDefault: false, locked: false });
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
  lockedScopes = [],
  grantedScopes,
  providerName,
}: UpstreamScopePickerProps) {
  const [customInput, setCustomInput] = useState("");
  const selected = new Set(value);
  const locked = new Set(lockedScopes);
  const pills = buildPills(catalog, defaultScopes, value, lockedScopes);

  // Edit-mode change summary (NyxID#917): when editing an existing connection,
  // diff the current selection against what's already granted. `labelFor` maps
  // a scope to its human pill label for readable copy.
  const labelFor = (scope: string) =>
    pills.find((p) => p.scope === scope)?.label ?? scope;
  const grantedSet = grantedScopes ? new Set(grantedScopes) : null;
  const added = grantedSet ? value.filter((s) => !grantedSet.has(s)) : [];
  const removed = grantedScopes
    ? grantedScopes.filter((s) => !selected.has(s))
    : [];
  const hasChanges = added.length > 0 || removed.length > 0;

  function toggle(scope: string) {
    // Locked (already-granted) scopes are append-only — can't be deselected.
    if (locked.has(scope)) return;
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
            const isOn = selected.has(p.scope) || p.locked;
            return (
              <button
                key={p.scope}
                type="button"
                aria-pressed={isOn}
                disabled={p.locked}
                title={
                  p.locked
                    ? `${p.description ?? p.scope} — already granted; can't be removed here`
                    : (p.description ?? p.scope)
                }
                onClick={() => {
                  toggle(p.scope);
                }}
                className={
                  "group inline-flex max-w-full items-center gap-1.5 rounded-full border px-3 py-1.5 text-left text-[12px] transition-colors " +
                  (p.locked
                    ? "cursor-default border-primary/60 bg-primary/10 text-foreground"
                    : isOn
                      ? "border-primary bg-primary/15 text-foreground"
                      : "border-border bg-transparent text-muted-foreground hover:border-primary/50 hover:bg-muted/40")
                }
              >
                {p.sensitive ? (
                  <>
                    {/* Warning-token status dot — DESIGN.md semantic
                        "warning/attention" (#F59E0B). Decorative; the
                        sr-only text + the legend below carry the meaning
                        so it isn't conveyed by color alone. */}
                    <span
                      aria-hidden="true"
                      className="h-1.5 w-1.5 shrink-0 rounded-full bg-warning"
                    />
                    <span className="sr-only">(write or admin access) </span>
                  </>
                ) : null}
                <span className="truncate">{p.label}</span>
                {p.locked ? (
                  <span className="shrink-0 text-[11px] text-muted-foreground">
                    granted
                  </span>
                ) : p.isDefault ? (
                  <span className="shrink-0 text-[11px] text-muted-foreground">
                    default
                  </span>
                ) : null}
                {isOn && !p.locked ? (
                  <X className="h-3 w-3 shrink-0 opacity-50 group-hover:opacity-100" />
                ) : null}
              </button>
            );
          })}
        </div>
      ) : null}

      {pills.some((p) => p.sensitive) ? (
        <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <span
            aria-hidden="true"
            className="h-1.5 w-1.5 shrink-0 rounded-full bg-warning"
          />
          Dot marks a write or admin-level scope.
        </p>
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
      {grantedSet && hasChanges ? (
        <div className="flex flex-col gap-1 rounded-lg border border-border bg-muted/40 px-3 py-2 text-[12px]">
          <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            Changes
          </span>
          {added.length > 0 ? (
            <p className="text-foreground">
              <span className="text-success">+ Adding:</span>{" "}
              {added.map(labelFor).join(", ")}
            </p>
          ) : null}
          {removed.length > 0 ? (
            <>
              <p className="text-foreground">
                <span className="text-destructive">− Removing:</span>{" "}
                {removed.map(labelFor).join(", ")}
              </p>
              <p className="text-[11px] text-warning">
                Removing a permission re-authorizes this connection and will
                stop any app that relies on it. NyxID will use only the
                remaining permissions; the old access at{" "}
                {providerName ?? "the provider"} stays until you revoke it
                there.
              </p>
            </>
          ) : null}
        </div>
      ) : null}

      <p className="text-xs text-muted-foreground">
        {locked.size > 0
          ? "Scopes marked “granted” are already authorized and locked — this provider can’t narrow them by re-authorizing, so they can’t be removed here. Add anything missing above."
          : grantedSet
            ? "Tick to add a permission, untick to remove one, then update. Changes re-authorize this connection at the provider."
            : pills.length > 0
              ? "Selected scopes are requested at sign-in. Defaults are pre-selected — deselect to drop one. Add anything missing above; the upstream provider decides whether to grant them."
              : "Comma- or space-separated. The upstream provider decides whether to grant them."}
      </p>
    </div>
  );
}
