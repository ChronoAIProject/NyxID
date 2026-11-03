import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useRouter } from "@tanstack/react-router";
import { GroupedDraftContext, useGroupedDraftGuard, type GroupedDraft } from "@/lib/grouped-drafts";
import { Button } from "./button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "./dialog";

export function GroupedDraftBoundary({ children }: { readonly children: ReactNode }) {
  const [entries, setEntries] = useState<ReadonlyMap<string, GroupedDraft>>(new Map());
  const current = useRef(entries);
  const [pending, setPending] = useState(false);
  const request = useRef<{ owner?: string; promise: Promise<boolean>; resolve: (discard: boolean) => void } | null>(null);
  const register = useCallback((id: string, entry: GroupedDraft) => {
    current.current = new Map(current.current).set(id, entry);
    setEntries(current.current);
  }, []);
  const remove = useCallback((id: string) => {
    const next = new Map(current.current);
    next.delete(id);
    current.current = next;
    setEntries(next);
  }, []);
  const confirmDiscard = useCallback((owner?: string) => {
    if (request.current) return request.current.promise;
    const dirty = [...current.current.values()].some((entry) => entry.dirty && (!owner || entry.owner === owner || entry.owner.startsWith(`${owner}/`)));
    if (!dirty) return Promise.resolve(true);
    let resolve!: (discard: boolean) => void;
    const promise = new Promise<boolean>((done) => { resolve = done; });
    request.current = { owner, promise, resolve };
    setPending(true);
    return promise;
  }, []);
  const finish = useCallback((discard: boolean) => {
    const active = request.current;
    if (!active) return;
    if (discard) {
      const dirty = [...current.current.values()].filter((entry) => entry.dirty && (!active.owner || entry.owner === active.owner || entry.owner.startsWith(`${active.owner}/`)));
      if (dirty.some((entry) => entry.pending)) return;
      dirty.forEach((entry) => entry.discard());
      const next = new Map(current.current);
      for (const [id, entry] of next) {
        if (dirty.includes(entry)) next.set(id, { ...entry, dirty: false });
      }
      current.current = next;
      setEntries(next);
    }
    request.current = null;
    setPending(false);
    active.resolve(discard);
  }, []);
  useEffect(() => () => {
    request.current?.resolve(false);
    request.current = null;
  }, []);
  return <GroupedDraftContext.Provider value={{ entries, register, remove, confirmDiscard, pending, finish }}>{children}</GroupedDraftContext.Provider>;
}

/** Render inside the sheet so the confirmation inherits its overlay layer. */
export function GroupedDraftDialog() {
  const guard = useGroupedDraftGuard();
  if (!guard) return null;
  const busy = [...guard.entries.values()].some((entry) => entry.dirty && entry.pending);
  return <Dialog open={guard.pending} onOpenChange={(open) => { if (!open) guard.finish(false); }}>
    <DialogContent>
      <DialogHeader>
        <DialogTitle>Discard unsaved changes?</DialogTitle>
        <DialogDescription>Your changes have not been saved.</DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button variant="ghost" autoFocus onClick={() => guard.finish(false)}>Keep editing</Button>
        <Button variant="destructive" disabled={busy} onClick={() => guard.finish(true)}>Discard</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}

export function GroupedDraftNavigation({ active }: { readonly active: boolean }) {
  const router = useRouter({ warn: false });
  const guard = useGroupedDraftGuard();
  const confirm = guard?.confirmDiscard;
  const dirty = [...(guard?.entries.values() ?? [])].some((entry) => entry.dirty);
  useEffect(() => {
    if (!active || !router || !confirm) return;
    return router.history.block({
      blockerFn: async ({ action, currentLocation, nextLocation }) => {
        const current = new URLSearchParams(currentLocation.search);
        const next = new URLSearchParams(nextLocation.search);
        // A bare agent link adopts its latest thread asynchronously. That
        // canonical replacement retains this drawer and every draft in it.
        const sameDrawer = action === "REPLACE" &&
          currentLocation.pathname === nextLocation.pathname &&
          current.get("agent") && current.get("details") &&
          current.get("agent") === next.get("agent") &&
          current.get("details") === next.get("details") &&
          current.get("proposal") === next.get("proposal");
        return sameDrawer ? false : !(await confirm());
      },
      enableBeforeUnload: () => dirty,
    });
  }, [active, router, confirm, dirty]);
  return null;
}
