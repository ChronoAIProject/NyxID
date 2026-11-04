import { flushSync } from "react-dom";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogBody, DialogFooter } from "./dialog";
import { Button } from "./button";
import { GroupedList, GroupedRow } from "./grouped-list";
import { afterGroupedFormCommit, type GroupedFormEntry } from "@/lib/grouped-form";
import type { FormChange } from "@/lib/grouped-form-changes";
import { DrawerSectionInfoContext } from "@/lib/drawer-section-info";

function ReviewValue({ children }: { readonly children: string }) {
  const [expanded, setExpanded] = useState(false);
  return <span className="block min-w-0 break-words whitespace-pre-wrap">
    <span className={!expanded ? "line-clamp-3" : undefined}>{children}</span>
    {children.length > 160 && <button type="button" className="mt-1 text-11 font-medium text-primary-text" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>{expanded ? "Show less" : "Show more"}</button>}
  </span>;
}
function Change({ change }: { readonly change: FormChange }) {
  return <GroupedRow className="items-start">
    <div className="min-w-0 flex-1 space-y-1">
      <p>{change.label}</p>
      <div className="text-11 font-normal text-text-tertiary">
        {change.added ? <>{change.added.map((value) => <p key={`+${value}`}>+ {value}</p>)}{change.removed.map((value) => <p key={`-${value}`}>− {value}</p>)}</> : <div className="space-y-1"><ReviewValue>{change.before}</ReviewValue><span className="block">{" → "}</span><ReviewValue>{change.after}</ReviewValue></div>}
      </div>
    </div>
  </GroupedRow>;
}

/** Reads registrations again after the save commit, so partial successes name only cleared drafts. */
export function ChangeReview({ entries, save, disabled, children }: {
  readonly entries: readonly GroupedFormEntry[];
  readonly save: () => Promise<void | readonly string[]>;
  readonly disabled?: boolean;
  readonly children: (review: () => void) => ReactNode;
}) {
  const [review, setReview] = useState<{ label: string; changes: readonly FormChange[] }[] | null>(null);
  const [saving, setSaving] = useState(false);
  const latest = useRef(entries);
  useLayoutEffect(() => { latest.current = entries; });
  return <>
    {children(() => { const dirty = entries.filter((entry) => entry.dirty); if (dirty.length && !saving) setReview(dirty.map((entry) => ({ label: entry.label, changes: entry.changes?.() ?? [] }))); })}
    <Dialog open={review !== null} onOpenChange={(open) => { if (!open && !saving) setReview(null); }}>
      <DialogContent scrollMode="body" className="bg-background">
        <DialogHeader><DialogTitle>Review changes</DialogTitle><DialogDescription>Check what will be saved before continuing.</DialogDescription></DialogHeader>
        <DialogBody className="space-y-6 py-4"><DrawerSectionInfoContext value={false}>
          {review?.map((section, index) => <GroupedList key={`${section.label}-${index}`} title={section.label}>
            {section.changes.length ? section.changes.map((change, index) => <Change key={`${change.label}-${index}`} change={change} />) : <GroupedRow title="Save changes to this section" />}
          </GroupedList>)}
        </DrawerSectionInfoContext></DialogBody>
        <DialogFooter className="border-t border-border/50 bg-background">
          <Button type="button" variant="ghost" disabled={saving} onClick={() => setReview(null)}>Keep editing</Button>
          <Button type="button" variant="primary" disabled={saving || disabled} onClick={async () => {
            const attempted = entries.filter((entry) => entry.dirty);
            setSaving(true);
            try {
              const result = await save();
              flushSync(() => {});
              await afterGroupedFormCommit();
              const saved = [...new Set(result ?? attempted.filter((entry) => entry.changes ? entry.changes().length === 0 : !latest.current.some((current) => current.label === entry.label && current.dirty)).map((entry) => entry.label))];
              if (saved.length) toast.success(`Saved ${new Intl.ListFormat("en", { style: "long", type: "conjunction" }).format(saved)}`);
            } catch { /* The form keeps its own error and draft. */ }
            finally { setReview(null); setSaving(false); }
          }}>{saving ? "Saving…" : "Save changes"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </>;
}
