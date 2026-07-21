import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ExternalLink, KeyRound, RefreshCw } from "lucide-react";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useKeys } from "@/hooks/use-keys";
import type { ConnectCardContentBlock } from "@/types/assistant";

const STATE_LABEL: Record<ConnectCardContentBlock["state"], string> = {
  needs_connection: "Not connected",
  waiting_for_provider: "Authorizing",
  waiting_for_user: "Waiting for you",
  connected: "Connected",
  error: "Failed",
  timed_out: "Timed out",
};

/**
 * Compact one-row connection prompt (DESIGN.md banner anatomy: icon tile +
 * text + action, compact density). The icon comes from the same catalog
 * glyph registry the AI Services page uses — `catalog_slug` is the raw
 * NyxID service slug (`api-github`, `llm-openai`) passed straight through.
 *
 * New connections and OAuth reauthorization happen in place through the
 * shared AddKeyDialog. Existing non-OAuth credentials route to their key
 * detail because credential rotation is owned by that management surface.
 */
export function ConnectCard({
  block,
}: {
  readonly block: ConnectCardContentBlock;
}) {
  const navigate = useNavigate();
  const [dialogOpen, setDialogOpen] = useState(false);
  const { data: keys } = useKeys();
  const matchingKey =
    (block.key_id
      ? (keys ?? []).find((key) => key.id === block.key_id)
      : undefined) ??
    (keys ?? []).find((key) => key.catalog_service_slug === block.catalog_slug);
  const needsReauthorization = block.reason_code === "NYXID_UNAUTHORIZED";
  const canManageMatchingKey =
    matchingKey !== undefined &&
    !matchingKey.auto_connected &&
    !(
      matchingKey.credential_source?.type === "org" &&
      matchingKey.credential_source.role !== "admin"
    );
  const reconnectKey =
    needsReauthorization &&
    canManageMatchingKey &&
    (matchingKey.credential_type === "oauth2" ||
      matchingKey.auth_method === "oauth2" ||
      matchingKey.auth_method === "oidc")
      ? matchingKey
      : null;
  const connectedNow =
    !needsReauthorization &&
    block.catalog_slug !== "custom" &&
    (keys ?? []).some(
      (key) => key.is_active && key.catalog_service_slug === block.catalog_slug,
    );
  const connected = connectedNow || block.state === "connected";
  const failed =
    !connected && (block.state === "error" || block.state === "timed_out");
  const guidance = connected
    ? "Connected — send your request again."
    : (block.error_message ?? block.steps[0]?.body ?? block.subtitle);
  const actionLabel = reconnectKey
    ? "Reconnect"
    : needsReauthorization && matchingKey
      ? "Manage"
      : "Connect";
  const stateLabel = needsReauthorization
    ? "Reauthorization required"
    : STATE_LABEL[block.state];

  return (
    <section className="flex items-center gap-3 rounded-xl border border-border/70 bg-card px-4 py-3">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
        <ServiceIcon slug={block.catalog_slug} size="sm" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <p className="truncate text-[13px] font-semibold text-foreground">
            {block.service_name}
          </p>
          <Badge
            variant={connected ? "success" : failed ? "destructive" : "warning"}
          >
            {connected ? "Connected" : stateLabel}
          </Badge>
        </div>
        <p className="truncate text-[11px] text-muted-foreground">{guidance}</p>
      </div>
      {!connected && (
        <Button
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick={() => {
            if (needsReauthorization && matchingKey && !reconnectKey) {
              void navigate({
                to: "/keys/$keyId",
                params: { keyId: matchingKey.id },
              });
              return;
            }
            setDialogOpen(true);
          }}
        >
          {reconnectKey ? (
            <RefreshCw />
          ) : block.auth_kind === "api_key" ? (
            <KeyRound />
          ) : (
            <ExternalLink />
          )}
          {actionLabel}
        </Button>
      )}
      <AddKeyDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        prefillSlug={
          !reconnectKey && block.catalog_slug !== "custom"
            ? block.catalog_slug
            : undefined
        }
        reconnectKey={reconnectKey}
      />
    </section>
  );
}
