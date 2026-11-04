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

export function GroupedFormScope({
  children,
  inherit = false,
}: {
  readonly children: ReactNode;
  readonly inherit?: boolean;
}) {
  const parent = useContext(GroupedFormContext);
  const [entries, setEntries] = useState<ReadonlyMap<string, GroupedFormEntry>>(
    new Map(),
  );
  const [saving, setSaving] = useState(false);
  const currentEntries = useRef(entries);
  const lock = useRef(false);
  const register = useCallback((id: string, entry: GroupedFormEntry) => {
    currentEntries.current = new Map(currentEntries.current).set(id, entry);
    setEntries(currentEntries.current);
  }, []);
  const remove = useCallback((id: string) => {
    const next = new Map(currentEntries.current);
    next.delete(id);
    currentEntries.current = next;
    setEntries(next);
  }, []);
  async function save() {
    if (lock.current) return;
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
      value={{ entries, register, remove, saving, save }}
    >
      {children}
    </GroupedFormContext.Provider>
  );
}

export function GroupedFormBar() {
  const context = useContext(GroupedFormContext);
  const dirty = [...(context?.entries.values() ?? [])].filter(
    (entry) => entry.dirty,
  );
  if (!context || !dirty.length) return null;
  const busy = context.saving || dirty.some((entry) => entry.pending);
  return (
    <GroupedSaveBar role="region" aria-label="Unsaved changes">
      <p
        className="mr-auto basis-full text-11 text-text-tertiary"
        aria-live="polite"
      >
        Unsaved: {[...new Set(dirty.map((entry) => entry.label))].join(", ")}
      </p>
      <Button
        type="button"
        variant="ghost"
        disabled={busy}
        onClick={() => dirty.forEach((entry) => entry.discard())}
      >
        Discard
      </Button>
      <Button
        type="button"
        variant="primary"
        disabled={busy || dirty.some((entry) => entry.disabled)}
        onClick={() => void context.save()}
      >
        Save changes
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
