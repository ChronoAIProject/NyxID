import { AlertTriangle } from "lucide-react";
import {
  GRANT_CASCADE_CAVEAT,
  type GrantCascadeDetails,
} from "@/schemas/oauth-revocation";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function GrantCascadeDialog({
  details,
  isPending,
  onCascade,
  onRemoveOnly,
  onCancel,
}: {
  readonly details: GrantCascadeDetails;
  readonly isPending: boolean;
  readonly onCascade: () => void;
  readonly onRemoveOnly: () => void;
  readonly onCancel: () => void;
}) {
  const affectedCount = details.siblings.length + 1;

  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onCancel())}>
      <DialogContent className="md:max-w-xl">
        <DialogHeader>
          <div className="flex items-start gap-3 pr-6">
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive">
              <AlertTriangle className="h-4 w-4" />
            </span>
            <div className="space-y-1.5">
              <DialogTitle>
                Disconnect {details.provider_name} everywhere?
              </DialogTitle>
              <DialogDescription>
                This upstream authorization is shared by other NyxID services.
                Choose how broadly to disconnect it.
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <div className="space-y-4 text-[12px]">
          <div className="rounded-lg border border-destructive/20 bg-destructive/[0.04] px-4 py-3">
            <p className="font-medium text-foreground">
              Also using this {details.provider_name} authorization
            </p>
            <ul className="mt-2 space-y-1.5 text-muted-foreground">
              {details.siblings.map((sibling) => (
                <li key={sibling.user_service_id} className="flex gap-2">
                  <span aria-hidden="true">-</span>
                  <span>{sibling.name}</span>
                </li>
              ))}
            </ul>
          </div>

          {details.unaffected_other_app.length > 0 && (
            <div className="space-y-1.5 text-muted-foreground">
              {details.unaffected_other_app.map((service) => (
                <p key={service.user_service_id}>
                  not affected (different OAuth app): {service.name}
                </p>
              ))}
            </div>
          )}

          {!details.token_scope_available && (
            <p className="rounded-lg border border-warning/20 bg-warning/[0.04] px-4 py-3 text-warning">
              Removing only this service cannot revoke its upstream access. The
              authorization will remain active at {details.provider_name}.
            </p>
          )}

          <p className="text-[11px] leading-relaxed text-text-tertiary">
            {GRANT_CASCADE_CAVEAT}
          </p>
        </div>

        <DialogFooter className="flex-col md:flex-col md:items-stretch md:space-x-0">
          <Button variant="outline" disabled={isPending} onClick={onCancel}>
            Cancel
          </Button>
          <Button
            variant="outline"
            className="whitespace-normal"
            disabled={isPending}
            onClick={onRemoveOnly}
          >
            {details.token_scope_available
              ? "Remove only this service"
              : "Remove from NyxID only"}
          </Button>
          <Button
            variant="destructive"
            className="whitespace-normal"
            isLoading={isPending}
            onClick={onCascade}
          >
            Disconnect {details.provider_name} everywhere ({affectedCount}{" "}
            services)
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
