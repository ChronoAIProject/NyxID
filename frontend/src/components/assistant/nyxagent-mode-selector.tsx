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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { NyxAgentAccessMode } from "@/schemas/assistant-nyxagent";

export function NyxAgentModeSelector({
  mode,
  disabled,
  onChange,
}: {
  readonly mode: NyxAgentAccessMode;
  readonly disabled: boolean;
  readonly onChange: (mode: NyxAgentAccessMode) => Promise<unknown>;
}) {
  const [confirm, setConfirm] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();

  async function change(value: NyxAgentAccessMode) {
    setSaving(true);
    setError(undefined);
    try {
      await onChange(value);
      setConfirm(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not change access mode.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="min-w-0 flex-1">
      <label
        htmlFor="nyxagent-mode"
        className="mb-1 block text-[10px] font-medium text-text-tertiary"
      >
        Mode
      </label>
      <Select
        value={mode}
        disabled={disabled || saving}
        onValueChange={(value) => {
          if (value === mode) return;
          setError(undefined);
          if (value === "full") setConfirm(true);
          else void change("ask");
        }}
      >
        <SelectTrigger id="nyxagent-mode" className="h-7 rounded-md px-2">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="ask" textValue="Ask before acting">
            <div>Ask before acting</div>
            <div className="text-[11px] text-muted-foreground">
              Review permissions and deletions.
            </div>
          </SelectItem>
          <SelectItem value="full" textValue="Full access">
            <div>Full access</div>
            <div className="text-[11px] text-muted-foreground">
              Act across your account without cards.
            </div>
          </SelectItem>
        </SelectContent>
      </Select>
      {!confirm && error ? (
        <p role="alert" className="text-[11px] text-destructive">
          {error}
        </p>
      ) : null}
      <Dialog
        open={confirm}
        onOpenChange={(open) => {
          if (!saving) setConfirm(open);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Allow full access for this chat?</DialogTitle>
            <DialogDescription>
              This chat may use all your connected services and nodes, change agent keys, channel
              bots, services, nodes and approval settings, and delete resources without further
              permission cards. Creating or rotating keys, entering secrets, deciding approvals,
              organization administration and billing remain in the NyxID UI.
            </DialogDescription>
          </DialogHeader>
          {error ? (
            <p role="alert" className="text-[12px] text-destructive">
              {error}
            </p>
          ) : null}
          <DialogFooter>
            <Button variant="outline" disabled={saving} onClick={() => setConfirm(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={disabled || saving}
              onClick={() => void change("full")}
            >
              Enable full access
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
