import { flushSync } from "react-dom";
import {
  useCallback,
  useContext,
  useRef,
  useLayoutEffect,
  useState,
  type ReactNode,
} from "react";
import {
  useGroupedForm,
  afterGroupedFormCommit,
  GroupedFormContext,
  type GroupedFormEntry,
} from "@/lib/grouped-form";
import { ChangeReview } from "./change-review";
import { GroupedSaveBar } from "./grouped-list";
import { Button } from "./button";
import { GroupedDialogCancel } from "./grouped-dialog";
import { useGroupedDraftGuard } from "@/lib/grouped-drafts";

export function GroupedFormScope({
  children,
  inherit = false,
  collect = false,
  viewport = false,
}: {
  readonly children: ReactNode;
  readonly inherit?: boolean;
  readonly collect?: boolean;
  readonly viewport?: boolean;
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
    if (lock.current || (collect && parent?.saving)) return [];
    lock.current = true;
    setSaving(true);
    const saved: string[] = [];
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
          flushSync(() => {});
          await afterGroupedFormCommit();
          const current = currentEntries.current.get(id);
          if (!current || !(current.isDirty?.() ?? current.dirty)) saved.push(entry.label);
        } catch {
          /* The form renders its own request error. */
        }
      }
    } finally {
      lock.current = false;
      setSaving(false);
    }
    return saved;
  }
  if (inherit && parent) return children;
  return (
    <ChangeReview entries={[...entries.values()]} save={save} disabled={saving || [...entries.values()].some((entry) => entry.dirty && (entry.disabled || entry.pending))}>{(requestReview) => <GroupedFormContext.Provider
      value={{ entries, register, remove, requestReview: collect && parent?.viewport ? parent.requestReview : requestReview, viewport: viewport || Boolean(collect && parent?.viewport), saving: saving || Boolean(collect && parent?.saving), save }}
    >
      {children}
    </GroupedFormContext.Provider>}</ChangeReview>
  );
}

export function GroupedFormBar({ dialog = false, viewport = false }: { readonly dialog?: boolean; readonly viewport?: boolean }) {
  const context = useContext(GroupedFormContext);
  const dirty = [...(context?.entries.values() ?? [])].filter(
    (entry) => entry.dirty,
  );
  if (!context || (context.viewport && !viewport) || (!dialog && !dirty.length)) return null;
  const busy = context.saving || dirty.some((entry) => entry.pending);
  return (
    <GroupedSaveBar
      data-viewport-save-bar={viewport ? "" : undefined}
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
        onClick={context.requestReview}
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
  const context = useContext(GroupedFormContext);
  const guard = useGroupedDraftGuard();
  return (
    <ChangeReview entries={[entry]} save={entry.save} disabled={entry.disabled || entry.pending}>{(review) => <>
      {intent === "edit" && <ReviewSubmit review={context?.requestReview ?? review} />}
      {!coordinated && <GroupedSaveBar role="region" aria-label="Form actions">
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
        onClick={() => { if (intent === "create") void entry.save(); else review(); }}
      >
        {saveLabel}
      </Button>
    </GroupedSaveBar>}
    </>}</ChangeReview>
  );
}

export function GroupedFormActions({
  changes,
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
    changes,
    label,
    dirty,
    pending,
    disabled,
    save,
    discard,
  });
  const context = useContext(GroupedFormContext);
  return (
    <ChangeReview entries={[{label, dirty, pending, disabled, save, discard, changes}]} save={save} disabled={disabled || pending}>{(review) => <>
    <ReviewSubmit review={context?.requestReview ?? review} />
    {!coordinated && dirty && <GroupedSaveBar role="region" aria-label="Unsaved changes">
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
        onClick={review}
      >
        {saveLabel}
      </Button>
    </GroupedSaveBar>}
    </>}</ChangeReview>
  );
}

/** Route native Enter submissions through the same review as the visible action. */
function ReviewSubmit({ review }: { readonly review: () => void }) {
  const marker = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const form = marker.current?.closest("form");
    if (!form) return;
    const intercept = (event: Event) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      review();
    };
    form.addEventListener("submit", intercept, true);
    return () => form.removeEventListener("submit", intercept, true);
  }, [review]);
  return <span ref={marker} hidden data-review-submit=""><button type="submit" tabIndex={-1} aria-hidden /></span>;
}
