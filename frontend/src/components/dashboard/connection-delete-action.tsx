import { translateErrorMessage } from "@/lib/error-messages";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { GrantCascadeDialog } from "@/components/shared/grant-cascade-dialog";
import { useDeleteKey, type DeleteKeyInput } from "@/hooks/use-keys";
import { ApiError } from "@/lib/api-client";
import {
  parseGrantCascadeDetails,
  type GrantCascadeDetails,
} from "@/schemas/oauth-revocation";
import type { KeyInfo } from "@/types/keys";

export function ConnectionDeleteAction({
  connection,
  disabled,
}: {
  readonly connection: KeyInfo;
  readonly disabled?: boolean;
}) {
  const { t } = useUiTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        disabled={disabled}
        aria-label={t("Delete connection {{label}} ({{slug}})", { label: connection.label, slug: connection.slug })}
        onClick={() => setOpen(true)}
      >
        <Trash2 className="size-3" aria-hidden="true" />{t(" Delete")}</Button>
      {open && (
        <DeleteConnectionDialog
          connection={connection}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

function DeleteConnectionDialog({
  connection,
  onClose,
}: {
  readonly connection: KeyInfo;
  readonly onClose: () => void;
}) {
  const { t } = useUiTranslation();
  const deletion = useDeleteKey();
  const [cascade, setCascade] = useState<GrantCascadeDetails | null>(null);
  const close = () => {
    if (!deletion.isPending) onClose();
  };
  const remove = (input: string | DeleteKeyInput) =>
    deletion.mutate(input, {
      onSuccess: (response) => {
        toast.success(
          response.upstream_revocation_scheduled
            ? "Removed from NyxID. Upstream revocation scheduled."
            : "Removed from NyxID. Upstream access remains active.",
        );
        onClose();
      },
      onError: (error) => {
        const details =
          error instanceof ApiError
            ? parseGrantCascadeDetails(error.errorResponse)
            : null;
        if (details) setCascade(details);
        else
          toast.error(
            translateErrorMessage(error instanceof ApiError
              ? error.message
              : "Failed to delete connection"),
          );
      },
    });
  if (cascade)
    return (
      <GrantCascadeDialog
        details={cascade}
        isPending={deletion.isPending}
        onCancel={close}
        onCascade={() => remove({ keyId: connection.id, cascadeGrant: true })}
        onRemoveOnly={() =>
          remove({ keyId: connection.id, grantScope: "token" })
        }
      />
    );
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("Delete connection")}</DialogTitle>
          <DialogDescription>{t("Delete ")}{connection.label} ({connection.slug}{t(") and its stored credential? Requests using this connection will stop working. This cannot be undone. To stop it temporarily, use Disable. Removing an OAuth connection may also revoke its upstream authorization; shared authorizations require a separate confirmation.")}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={deletion.isPending}
            onClick={close}
          >{t("Cancel")}</Button>
          <Button
            type="button"
            variant="destructive"
            isLoading={deletion.isPending}
            disabled={deletion.isPending}
            onClick={() => remove(connection.id)}
          >{t("Delete connection")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
