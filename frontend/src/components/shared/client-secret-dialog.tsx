import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

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
  const { t } = useUiTranslation();
  function handleCopy(value: string, label: string) {
    void copyToClipboard(value)
      .then(() => toast.success(translateUi("{{label}} copied", { label: label })))
      .catch(() => toast.error(translateUi("Failed to copy {{label}}", { label: label })));
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {clientId ? t("Save Client Secret") : t("New Client Secret")}
          </DialogTitle>
          <DialogDescription>{t("This secret is shown only once. Copy and store it securely now.")}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          {clientId && (
            <div className="space-y-2">
              <p className="text-xs uppercase tracking-wide text-text-tertiary">{t("Client ID")}</p>
              <div className="flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2">
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
            <p className="text-xs uppercase tracking-wide text-text-tertiary">{t("Client Secret")}</p>
            <div className="flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2">
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
          <p className="text-xs text-muted-foreground">
            <strong>{t("Lost it?")}</strong>{t(" If you don't copy this now, you'll need to rotate the credential to get a new one. (Old value stops working immediately on rotate.)")}</p>
        </div>
        <DialogFooter>
          <Button variant="primary" type="button" onClick={() => onOpenChange(false)}>{t("I have saved it")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
