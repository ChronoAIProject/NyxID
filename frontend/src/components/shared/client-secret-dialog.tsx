import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
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
  useUiTranslation();
  function handleCopy(value: string, label: string) {
    void copyToClipboard(value)
      .then(() => toast.success(translateUi("{{value1}} copied", { value1: label })))
      .catch(() => toast.error(translateErrorMessage(`Failed to copy ${label}`)));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {clientId ? translateUi("Save Client Secret") : translateUi("New Client Secret")}
          </DialogTitle>
          <DialogDescription>{translateUi("This secret is shown only once. Copy and store it securely now.")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          {clientId && (
            <div className="space-y-2">
              <p className="text-13 uppercase tracking-wide text-text-tertiary">{translateUi("Client ID")}</p>
              <div className="flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2">
                <p className="min-w-0 flex-1 truncate font-mono text-13">
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
            <p className="text-13 uppercase tracking-wide text-text-tertiary">{translateUi("Client Secret")}</p>
            <div className="flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2">
              <p className="min-w-0 flex-1 truncate font-mono text-13">
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
          <p className="text-13 text-muted-foreground">
            <strong>{translateUi("Lost it?")}</strong>{translateUi(" If you don't copy this now, you'll need to rotate the credential to get a new one. (Old value stops working immediately on rotate.)")}</p>
        </div>
        <DialogFooter>
          <Button variant="primary" type="button" onClick={() => onOpenChange(false)}>{translateUi("I have saved it")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
