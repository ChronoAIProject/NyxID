import { useState } from "react";
import { ArrowRightLeft } from "lucide-react";
import { useAuthStore } from "@/stores/auth-store";
import { useOwnershipTransferAuthorization } from "@/hooks/use-ownership-transfers";
import type {
  OwnershipResource,
  OwnershipResourceKind,
} from "@/types/ownership-transfers";
import { Button } from "@/components/ui/button";
import { DetailSection } from "./detail-section";
import { OwnershipTransferDialog } from "./ownership-transfer-dialog";

interface OwnershipTransferCardProps {
  readonly kind: OwnershipResourceKind;
  readonly resource: OwnershipResource;
  readonly onTransferred?: () => void;
}

export function OwnershipTransferCard(props: OwnershipTransferCardProps) {
  const user = useAuthStore((state) => state.user);
  const authorization = useOwnershipTransferAuthorization(
    props.kind,
    props.resource.id,
    user?.id,
  );
  if (!authorization.data?.can_transfer) return null;

  return (
    <TransferCard
      key={`${user?.id}:${props.kind}:${props.resource.id}:${props.resource.owner_user_id}`}
      {...props}
    />
  );
}

function TransferCard({
  kind,
  resource,
  onTransferred,
}: OwnershipTransferCardProps) {
  const [open, setOpen] = useState(false);

  return (
    <DetailSection title="Ownership transfer">
      <div className="flex flex-col gap-4 p-5 sm:flex-row sm:items-start sm:justify-between">
        <div className="max-w-2xl space-y-2 text-xs text-muted-foreground">
          <p>
            Transfer {kind === "service" ? "the catalog definition for " : ""}
            {resource.name} to another person or organization.
          </p>
          <p>
            {kind === "channel_bot"
              ? "Credentials and webhook settings stay with the bot. Existing routes will be retired, and the new owner must assign their own agents. Conversation history stays with the previous owner."
              : "Existing connections, credentials and pricing stay in place. Shared service configuration remains managed by NyxID admins."}
          </p>
        </div>
        <Button
          variant="outline"
          className="shrink-0"
          onClick={() => setOpen(true)}
        >
          <ArrowRightLeft className="mr-2 h-3.5 w-3.5" aria-hidden="true" />
          Transfer ownership
        </Button>
      </div>
      {open && (
        <OwnershipTransferDialog
          kind={kind}
          resource={resource}
          onClose={() => setOpen(false)}
          onTransferred={onTransferred}
        />
      )}
    </DetailSection>
  );
}
