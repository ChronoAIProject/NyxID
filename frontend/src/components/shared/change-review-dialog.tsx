import { toast } from "sonner";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { FormChange } from "@/lib/form-changes";

export function useChangeReview<T>(
  save: (payload: T) => Promise<void>,
  stale = false,
) {
  const [pending, setPending] = useState<{
    payload: T;
    changes: FormChange[];
  } | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function review(payload: T, changes: FormChange[]) {
    if (stale) return;
    if (!changes.length) {
      toast.info("No changes to save");
      return;
    }
    setError(null);
    setPending({ payload: structuredClone(payload), changes });
  }

  async function confirm() {
    if (!pending || saving || stale) return;
    setSaving(true);
    try {
      await save(pending.payload);
      setPending(null);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Unable to save changes",
      );
    } finally {
      setSaving(false);
    }
  }

  const dialog = (
    <Dialog
      open={pending !== null}
      onOpenChange={(open) => {
        if (!open && !saving) setPending(null);
      }}
    >
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Review changes</DialogTitle>
          <DialogDescription>
            Confirm these changes before saving.
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] space-y-4 overflow-y-auto">
          {pending?.changes.map((change) => (
            <div
              key={change.field}
              className="space-y-2 rounded-md border border-border p-3 text-sm"
            >
              <p className="font-medium">{change.field}</p>
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-xs text-muted-foreground">Current</p>
                  <pre className="whitespace-pre-wrap break-all font-sans">
                    {change.before}
                  </pre>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">New</p>
                  <pre className="whitespace-pre-wrap break-all font-sans">
                    {change.after}
                  </pre>
                </div>
              </div>
            </div>
          ))}
        </div>
        {stale && (
          <p role="alert">
            Saved values changed while you were editing. Cancel and load the
            latest values before saving.
          </p>
        )}
        {error && (
          <p role="alert" className="text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={saving}
            onClick={() => setPending(null)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            variant="primary"
            isLoading={saving}
            disabled={stale || saving}
            onClick={() => void confirm()}
          >
            Confirm changes
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
  return { review, dialog, cancel: () => setPending(null) };
}
