import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { copyToClipboard } from "@/lib/utils";
import { Copy } from "lucide-react";
import { toast } from "sonner";

interface ClientSecretDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly clientId?: string;
  readonly clientSecret: string;
}

export function ClientSecretDialog({
  open,
  onOpenChange,
  clientId,
  clientSecret,
}: ClientSecretDialogProps) {
  function handleCopy(value: string, label: string) {
    void copyToClipboard(value)
      .then(() => toast.success(`${label} copied`))
      .catch(() => toast.error(`Failed to copy ${label}`));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {clientId ? "Save Client Secret" : "New Client Secret"}
          </DialogTitle>
          <DialogDescription>
            This secret is shown only once. Copy and store it securely now.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          {clientId && (
            <div className="space-y-2">
              <p className="text-xs uppercase tracking-wide text-text-tertiary">
                Client ID
              </p>
              <div className="flex items-center gap-2 rounded-md border border-border bg-muted px-3 py-2">
                <p className="min-w-0 flex-1 truncate font-mono text-xs">
                  {clientId}
                </p>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  onClick={() => handleCopy(clientId, "Client ID")}
                >
                  <Copy className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
          )}
          <div className="space-y-2">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Client Secret
            </p>
            <div className="flex items-center gap-2 rounded-md border border-border bg-muted px-3 py-2">
              <p className="min-w-0 flex-1 truncate font-mono text-xs">
                {clientSecret}
              </p>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={() => handleCopy(clientSecret, "Client secret")}
              >
                <Copy className="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button type="button" onClick={() => onOpenChange(false)}>
            I have saved it
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
