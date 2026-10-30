import { useState } from "react";
import { AlertTriangle, RefreshCw } from "lucide-react";
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
import { DetailRow } from "@/components/shared/detail-row";
import { DetailSection } from "@/components/shared/detail-section";
import { Separator } from "@/components/ui/separator";
import { useResyncServiceIdentity } from "@/hooks/use-services";
import { ApiError } from "@/lib/api-client";
import type { DownstreamService, ResyncIdentityResponse } from "@/types/api";

function resyncResultLabel(
  result: Pick<ResyncIdentityResponse, "matched_count" | "changed_count">,
): string {
  return `${String(result.matched_count)} checked, ${String(result.changed_count)} updated`;
}

/**
 * Service detail section that copies this service's identity-forwarding
 * settings onto every connection to it, including customized ones. Renders
 * nothing (not even its separator) unless the server says the viewer may
 * (`can_resync_identity`); the server enforces the same rule.
 */
export function IdentityResyncSection({
  service,
}: {
  readonly service: Pick<
    DownstreamService,
    "id" | "name" | "can_resync_identity"
  >;
}) {
  const resync = useResyncServiceIdentity();
  const [confirming, setConfirming] = useState(false);
  const [result, setResult] = useState<ResyncIdentityResponse | null>(null);

  if (service.can_resync_identity !== true) return null;

  function handleConfirm() {
    resync.mutate(service.id, {
      onSuccess: (response) => {
        setResult(response);
        setConfirming(false);
        toast.success(`Resync finished: ${resyncResultLabel(response)}`);
      },
      onError: (error) => {
        toast.error(
          error instanceof ApiError
            ? error.message
            : "Failed to resync connection identity",
        );
      },
    });
  }

  return (
    <>
      <Separator />
      <DetailSection
        title="Connection identity"
        action={
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirming(true)}
          >
            <RefreshCw className="h-3 w-3" aria-hidden="true" />
            Resync connection identity
          </Button>
        }
      >
        <p className="px-4 py-2.5 text-12 text-muted-foreground">
          Each connection to this service keeps its own copy of the
          identity-forwarding settings. Editing the service updates connections
          that still match it and skips customized ones. Resync makes every
          connection match the service again.
        </p>
        {result && (
          <DetailRow label="Last resync" value={resyncResultLabel(result)} />
        )}
      </DetailSection>

      <Dialog
        open={confirming}
        onOpenChange={(open) => {
          if (!open && !resync.isPending) setConfirming(false);
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <div className="flex items-start gap-3 pr-6">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-warning/10 text-warning">
                <AlertTriangle className="h-4 w-4" aria-hidden="true" />
              </span>
              <div className="space-y-1.5">
                <DialogTitle>Resync connection identity?</DialogTitle>
                <DialogDescription>
                  This overwrites the identity-forwarding settings on every
                  connection to {service.name}, including connections whose
                  settings were customized, with this service&apos;s current
                  settings.
                </DialogDescription>
              </div>
            </div>
          </DialogHeader>

          <div className="space-y-2 text-12">
            <p className="text-muted-foreground">
              Overwritten: identity propagation mode and included user fields,
              JWT audience, access-token forwarding, and delegation-token
              injection and scope.
            </p>
            <p className="text-text-tertiary">
              Connections that already match are left unchanged. Stored
              credentials and sign-ins are not touched. This cannot be undone.
            </p>
          </div>

          <DialogFooter>
            <Button
              variant="outline"
              disabled={resync.isPending}
              onClick={() => setConfirming(false)}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              isLoading={resync.isPending}
              onClick={handleConfirm}
            >
              Resync
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
