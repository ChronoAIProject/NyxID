import { flushSync } from "react-dom";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogBody, DialogFooter } from "./dialog";
import { Button } from "./button";
import { GroupedList, GroupedRow } from "./grouped-list";
import { afterGroupedFormCommit, type GroupedFormEntry } from "@/lib/grouped-form";
import type { FormChange } from "@/lib/grouped-form-changes";
import { DrawerSectionInfoContext } from "@/lib/drawer-section-info";

function ReviewLine({ children, fullText }: { readonly children: ReactNode; readonly fullText: string }) {
  const [expanded, setExpanded] = useState(false);
  const [overflow, setOverflow] = useState(false);
  const value = useRef<HTMLParagraphElement>(null);
  useLayoutEffect(() => {
    const node = value.current;
    if (!node || expanded) return;
    const measure = () => setOverflow(node.scrollWidth > node.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [fullText, expanded]);
  return <div className="flex min-w-0 items-start gap-2">
    <p ref={value} data-review-line="" title={fullText} className={`min-w-0 flex-1 text-13 font-normal ${expanded ? "break-words whitespace-pre-wrap" : "truncate"}`}>{children}</p>
    {overflow && <button type="button" className="shrink-0 text-13 font-medium text-primary-text" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>{expanded ? "Show less" : "Show more"}</button>}
  </div>;
}
function Change({ change }: { readonly change: FormChange }) {
  const label = <span className="font-medium text-foreground">{change.label}: </span>;
  return <GroupedRow className="items-start">
    <div className="min-w-0 flex-1 space-y-1">
      {change.added ? <>
        {change.added.map((value) => <ReviewLine key={`+${value}`} fullText={`${change.label}: + ${value}`}>{label}<span className="text-foreground">+ {value}</span></ReviewLine>)}
        {change.removed.map((value) => <ReviewLine key={`-${value}`} fullText={`${change.label}: − ${value}`}>{label}<span className="text-text-tertiary">− {value}</span></ReviewLine>)}
      </> : <ReviewLine fullText={`${change.label}: ${change.before} → ${change.after}`}>
        {label}<span data-review-value="before" className="text-text-tertiary line-through">{change.before}</span><span className="text-text-tertiary"> → </span><span data-review-value="after" className="text-foreground">{change.after}</span>
      </ReviewLine>}
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
          {review?.map((section, index) => <GroupedList key={`${section.label}-${index}`} title={section.label.charAt(0).toLocaleUpperCase() + section.label.slice(1)}>
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
