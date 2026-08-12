import { AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { grantRevocationDescription } from "@/schemas/oauth-revocation";

/**
 * Confirmation for deleting a single connection.
 *
 * This exists because the explanation has to be read before the click, and
 * the connection modal had nowhere to put it: its footer confirm was a slot
 * sized for three words, so the grant-revoking copy introduced with upstream
 * revocation wrapped into two lines of prose sitting inside the card the user
 * was already looking at — indistinguishable from an error the app had just
 * raised at them.
 *
 * Named for the user-facing verb, not the backend mechanism: "delete" is the
 * one destructive action in this vocabulary, paired against the reversible
 * "disable". Upstream *revocation* is a consequence explained in the body
 * copy, and "revoked" separately survives as a credential *status* the same
 * card can display (`lib/credential-status.ts`) — keeping the button verb off
 * that word stops the two meanings colliding.
 *
 * `GrantCascadeDialog` is the sibling of this dialog for the case where the
 * backend reports other services sharing the same upstream grant; between
 * them every delete path now confirms in a modal.
 */
export function DeleteConnectionDialog({
  providerName,
  connectionLabel,
  revokesGrant,
  isPending,
  onConfirm,
  onCancel,
}: {
  readonly providerName: string;
  /** Shown only when a service has several connections and the name alone
   *  wouldn't say which one is about to go. */
  readonly connectionLabel?: string | null;
  /** Whether removing this connection also de-authorizes NyxID upstream. */
  readonly revokesGrant: boolean;
  readonly isPending: boolean;
  readonly onConfirm: () => void;
  readonly onCancel: () => void;
}) {
  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onCancel())}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <div className="flex items-start gap-3 pr-6">
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive">
              <AlertTriangle className="h-4 w-4" />
            </span>
            <div className="space-y-1.5">
              <DialogTitle>Delete {providerName} connection?</DialogTitle>
              <DialogDescription>
                {revokesGrant
                  ? grantRevocationDescription(providerName)
                  : `NyxID deletes its stored credential. Access you granted at ${providerName} stays active until you remove it there.`}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="space-y-2 text-[12px]">
          {connectionLabel && (
            <p className="text-muted-foreground">
              Connection:{" "}
              <span className="font-medium text-foreground">
                {connectionLabel}
              </span>
            </p>
          )}
          <p className="text-text-tertiary">
            The assistant loses access to this service immediately, and this
            cannot be undone. To stop it temporarily instead, disable it.
          </p>
        </div>

        <DialogFooter>
          <Button variant="outline" disabled={isPending} onClick={onCancel}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            isLoading={isPending}
            onClick={onConfirm}
          >
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
