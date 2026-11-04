import {
  useCallback,
  useContext,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  useGroupedForm,
  GroupedFormContext,
  type GroupedFormEntry,
} from "@/lib/grouped-form";
import { GroupedSaveBar } from "./grouped-list";
import { Button } from "./button";
import { GroupedDialogCancel } from "./grouped-dialog";
import { useGroupedDraftGuard } from "@/lib/grouped-drafts";

export function GroupedFormScope({
  children,
  inherit = false,
  collect = false,
}: {
  readonly children: ReactNode;
  readonly inherit?: boolean;
  readonly collect?: boolean;
}) {
  const parent = useContext(GroupedFormContext);
  const [entries, setEntries] = useState<ReadonlyMap<string, GroupedFormEntry>>(
    new Map(),
  );
  const [saving, setSaving] = useState(false);
  const currentEntries = useRef(entries);
  const lock = useRef(false);
  const parentRegister = collect ? parent?.register : undefined;
  const parentRemove = collect ? parent?.remove : undefined;
  const register = useCallback((id: string, entry: GroupedFormEntry) => {
    currentEntries.current = new Map(currentEntries.current).set(id, entry);
    setEntries(currentEntries.current);
    parentRegister?.(id, entry);
  }, [parentRegister]);
  const remove = useCallback((id: string) => {
    const next = new Map(currentEntries.current);
    next.delete(id);
    currentEntries.current = next;
    setEntries(next);
    parentRemove?.(id);
  }, [parentRemove]);
  async function save() {
    if (lock.current || (collect && parent?.saving)) return;
    lock.current = true;
    setSaving(true);
    try {
      const attempted = new Set<string>();
      while (true) {
        const next = [...currentEntries.current].find(
          ([id, entry]) => !attempted.has(id) && entry.dirty && !entry.disabled && !entry.pending,
        );
        if (!next) break;
        const [id, entry] = next;
        attempted.add(id);
        // A failure belongs to its form. Continue so independent forms can still succeed.
        try {
          await entry.save();
        } catch {
          /* The form renders its own request error. */
        }
      }
    } finally {
      lock.current = false;
      setSaving(false);
    }
  }
  if (inherit && parent) return children;
  return (
    <GroupedFormContext.Provider
      value={{ entries, register, remove, saving: saving || Boolean(collect && parent?.saving), save }}
    >
      {children}
    </GroupedFormContext.Provider>
  );
}

export function GroupedFormBar({ dialog = false }: { readonly dialog?: boolean }) {
  const context = useContext(GroupedFormContext);
  const dirty = [...(context?.entries.values() ?? [])].filter(
    (entry) => entry.dirty,
  );
  if (!context || (!dialog && !dirty.length)) return null;
  const busy = context.saving || dirty.some((entry) => entry.pending);
  return (
    <GroupedSaveBar
      role="region"
      aria-label={dialog ? "Form actions" : "Unsaved changes"}
    >
      {!dialog && (
        <p
          className="mr-auto basis-full text-11 text-text-tertiary"
          aria-live="polite"
        >
          Unsaved: {[...new Set(dirty.map((entry) => entry.label))].join(", ")}
        </p>
      )}
      {dialog ? (
        <GroupedDialogCancel disabled={busy}>Cancel</GroupedDialogCancel>
      ) : (
        <Button
          type="button"
          variant="ghost"
          disabled={busy}
          onClick={() => dirty.forEach((entry) => entry.discard())}
        >
          Discard
        </Button>
      )}
      <Button
        type="button"
        variant="primary"
        disabled={busy || !dirty.length || dirty.some((entry) => entry.disabled)}
        onClick={() => void context.save()}
      >
        Save changes
      </Button>
    </GroupedSaveBar>
  );
}

/** Dialog actions stay visible even when reviewed defaults are already valid. */
export function GroupedDialogActions({
  intent = "edit",
  cancel,
  saveLabel = "Save changes",
  saveVariant = "primary",
  ...entry
}: GroupedFormEntry & {
  readonly intent?: "create" | "edit";
  readonly cancel?: () => void;
  readonly saveLabel?: string;
  readonly saveVariant?: "primary" | "destructive";
}) {
  const coordinated = useGroupedForm(entry);
  const guard = useGroupedDraftGuard();
  if (coordinated) return null;
  return (
    <GroupedSaveBar role="region" aria-label="Form actions">
      {cancel ? (
        <Button
          type="button"
          variant="ghost"
          disabled={entry.pending}
          onClick={() => {
            if (!guard) cancel();
            else void guard.confirmDiscard().then((discard) => {
              if (discard) cancel();
            });
          }}
        >
          Cancel
        </Button>
      ) : (
        <GroupedDialogCancel disabled={entry.pending}>Cancel</GroupedDialogCancel>
      )}
      <Button
        type="button"
        variant={saveVariant}
        disabled={entry.disabled || entry.pending || (intent === "edit" && !entry.dirty)}
        onClick={() => void entry.save()}
      >
        {saveLabel}
      </Button>
    </GroupedSaveBar>
  );
}

export function GroupedFormActions({
  label,
  dirty,
  pending,
  disabled,
  save,
  discard,
  saveLabel = "Save changes",
  saveVariant = "primary",
}: GroupedFormEntry & {
  readonly saveLabel?: string;
  readonly saveVariant?: "primary" | "destructive";
}) {
  const coordinated = useGroupedForm({
    label,
    dirty,
    pending,
    disabled,
    save,
    discard,
  });
  if (coordinated || !dirty) return null;
  return (
    <GroupedSaveBar role="region" aria-label="Unsaved changes">
      <p className="mr-auto basis-full text-11 text-text-tertiary">
        Unsaved: {label}
      </p>
      <Button
        type="button"
        variant="ghost"
        disabled={pending}
        onClick={discard}
      >
        Discard
      </Button>
      <Button
        type="button"
        variant={saveVariant}
        disabled={disabled || pending}
        onClick={() => void save()}
      >
        {saveLabel}
      </Button>
    </GroupedSaveBar>
  );
}
