import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useNavigate } from "@tanstack/react-router";
import { useDeleteApiKey } from "@/hooks/use-api-keys";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { toast } from "sonner";

export function DeleteKeyDialog({
  open,
  onOpenChange,
  keyId,
  keyName,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly keyId: string;
  readonly keyName: string;
}) {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const deleteMutation = useDeleteApiKey();

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(keyId);
      toast.success(translateUi("API key revoked"));
      void navigate({ to: "/keys", search: { tab: "nyxid" } });
    } catch {
      toast.error(translateUi("Failed to revoke key"));
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("Revoke API Key")}</DialogTitle>
          <DialogDescription>{t("Are you sure you want to revoke \"")}{keyName}{t("\"? This action cannot be undone and any applications using this key will stop working.")}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>{t("Cancel")}</Button>
          <Button
            variant="destructive"
            onClick={() => void handleDelete()}
            disabled={deleteMutation.isPending}
          >{t("Revoke Key")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
