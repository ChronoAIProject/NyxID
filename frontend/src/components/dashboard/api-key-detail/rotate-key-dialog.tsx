import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useRotateApiKey } from "@/hooks/use-api-keys";
import { copyToClipboard } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";

export function RotateKeyDialog({
  open,
  onOpenChange,
  keyId,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly keyId: string;
}) {
  const navigate = useNavigate();
  const rotateMutation = useRotateApiKey();
  const [newKeyValue, setNewKeyValue] = useState<string | null>(null);
  const [newKeyId, setNewKeyId] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  async function handleRotate() {
    try {
      const result = await rotateMutation.mutateAsync(keyId);
      setNewKeyValue(result.full_key);
      setNewKeyId(result.id);
    } catch {
      toast.error("Failed to rotate key");
    }
  }

  async function handleCopy() {
    if (!newKeyValue) return;
    try {
      await copyToClipboard(newKeyValue);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy");
    }
  }

  function handleClose() {
    const rotatedKeyId = newKeyId;
    setNewKeyValue(null);
    setNewKeyId(null);
    setCopied(false);
    onOpenChange(false);
    if (rotatedKeyId) {
      void navigate({ to: "/keys/api-key/$keyId", params: { keyId: rotatedKeyId } });
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) handleClose();
      }}
    >
      <DialogContent>
        {newKeyValue ? (
          <>
            <DialogHeader>
              <DialogTitle>New API Key</DialogTitle>
              <DialogDescription>
                Copy your new API key now. You will not be able to see it again.
              </DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-2">
              <code className="flex-1 rounded-lg bg-muted p-3 font-mono text-[12px] break-all select-all">
                {newKeyValue}
              </code>
              <Button
                variant="outline"
                size="icon"
                onClick={() => void handleCopy()}
              >
                {copied ? (
                  <Check className="h-4 w-4 text-success" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </Button>
            </div>
            <DialogFooter>
              <Button variant="primary" onClick={handleClose}>Done</Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>Rotate API Key</DialogTitle>
              <DialogDescription>
                This will generate a new key and invalidate the old one. Any
                applications using the current key will stop working.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={handleClose}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={() => void handleRotate()}
                disabled={rotateMutation.isPending}
              >
                Rotate Key
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
