import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Copy, ExternalLink, Loader2, Lock } from "lucide-react";
import { toast } from "sonner";
import { ServiceIcon } from "@/components/service-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { useDeleteKey, useKey, useUpdateKey } from "@/hooks/use-keys";
import { formatDate } from "@/lib/utils";
import { connectorInitial } from "@/lib/assistant/plugins";
import type { KeyInfo } from "@/types/keys";

// Mirrors the Studio key-detail page status vocabulary exactly
// (key-detail.tsx:92): revoked/failed/refresh_failed are destructive.
function statusVariant(
  status: string,
): "success" | "destructive" | "secondary" {
  switch (status) {
    case "active":
      return "success";
    case "revoked":
    case "failed":
    case "refresh_failed":
      return "destructive";
    default:
      return "secondary";
  }
}

/**
 * Whether the current user may mutate this connection — the same rule the
 * full key-detail page uses (key-detail.tsx:2043): org-sourced credentials
 * are read-only to non-admin members, and auto-connected (platform-managed)
 * services can never be modified. Matches the backend's own authorization so
 * the modal never offers a control that would only error.
 */
function canModifyKey(key: KeyInfo): boolean {
  const source = key.credential_source;
  const readOnly = source?.type === "org" && source.role !== "admin";
  return !readOnly && !key.auto_connected;
}

function readOnlyReason(key: KeyInfo): string {
  if (key.auto_connected) {
    return "Platform-managed connection — manage it from the service catalog.";
  }
  return "Managed by your organization — an org admin can change it.";
}

function DetailRow({
  label,
  children,
}: {
  readonly label: string;
  readonly children: React.ReactNode;
}) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-2">
      <span className="shrink-0 text-[11px] uppercase tracking-[0.5px] text-text-tertiary">
        {label}
      </span>
      <span className="min-w-0 text-right text-[12px] text-foreground">
        {children}
      </span>
    </div>
  );
}

/**
 * Compact connection-management modal for the assistant Plugins surface: the
 * high-value actions (enable/disable, revoke) plus an escape hatch into the
 * full Studio key-detail page for rotation, routing, and scope edits.
 */
export function ManageConnectionModal({
  keyId,
  onClose,
}: {
  readonly keyId: string;
  readonly onClose: () => void;
}) {
  const { data: key, isLoading, error, refetch } = useKey(keyId);
  const updateKey = useUpdateKey();
  const deleteKey = useDeleteKey();
  const [confirmingRevoke, setConfirmingRevoke] = useState(false);

  function close() {
    setConfirmingRevoke(false);
    onClose();
  }

  function toggleActive(nextActive: boolean) {
    if (!key) return;
    updateKey.mutate(
      { keyId: key.id, is_active: nextActive },
      {
        onSuccess: () =>
          toast.success(nextActive ? "Connection enabled" : "Connection paused"),
        onError: () => toast.error("Could not update the connection."),
      },
    );
  }

  function revoke() {
    if (!key) return;
    deleteKey.mutate(key.id, {
      onSuccess: () => {
        toast.success("Connection revoked");
        close();
      },
      onError: () => toast.error("Could not revoke the connection."),
    });
  }

  const grantedScopes = key?.granted_scopes ?? null;
  const modifiable = key ? canModifyKey(key) : false;

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : close())}>
      <DialogContent className="max-w-md">
        {isLoading ? (
          <div className="flex h-40 items-center justify-center">
            <Loader2 className="h-4 w-4 animate-spin text-text-tertiary" />
          </div>
        ) : error || !key ? (
          <div className="flex h-40 flex-col items-center justify-center gap-3 text-center">
            <p className="text-[12px] text-muted-foreground">
              Couldn't load this connection.
            </p>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void refetch()}
            >
              Try again
            </Button>
          </div>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2.5">
                <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-muted text-[12px] font-semibold text-muted-foreground">
                  {key.catalog_service_slug ? (
                    <ServiceIcon slug={key.catalog_service_slug} size="md" />
                  ) : (
                    connectorInitial(key.label)
                  )}
                </span>
                <span className="min-w-0 truncate">{key.label}</span>
                <Badge variant={statusVariant(key.status)} className="ml-1">
                  {key.status.replaceAll("_", " ")}
                </Badge>
              </DialogTitle>
            </DialogHeader>

            <div className="divide-y divide-border/60">
              <DetailRow label="Credential">
                {key.credential_type.replaceAll("_", " ")}
              </DetailRow>
              {grantedScopes && grantedScopes.length > 0 && (
                <DetailRow label="Scopes">
                  <span className="flex flex-wrap justify-end gap-1">
                    {grantedScopes.map((scope) => (
                      <span
                        key={scope}
                        className="rounded bg-overlay-strong px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                      >
                        {scope}
                      </span>
                    ))}
                  </span>
                </DetailRow>
              )}
              {key.proxy_url && (
                <DetailRow label="Proxy URL">
                  <button
                    type="button"
                    onClick={() => {
                      void navigator.clipboard?.writeText(key.proxy_url ?? "");
                      toast.success("Proxy URL copied");
                    }}
                    className="inline-flex max-w-full items-center gap-1.5 font-mono text-[11px] text-muted-foreground transition-colors hover:text-foreground"
                  >
                    <span className="truncate">{key.proxy_url}</span>
                    <Copy className="h-3 w-3 shrink-0" />
                  </button>
                </DetailRow>
              )}
              <DetailRow label="Last used">
                {key.last_used_at ? formatDate(key.last_used_at) : "Never"}
              </DetailRow>
              {modifiable ? (
                <div className="flex items-center justify-between gap-4 py-2.5">
                  <div>
                    <p className="text-[12px] font-medium text-foreground">
                      Enabled
                    </p>
                    <p className="text-[11px] text-text-tertiary">
                      Pause to block the assistant from using this connection.
                    </p>
                  </div>
                  <Switch
                    checked={key.is_active}
                    disabled={updateKey.isPending}
                    onCheckedChange={toggleActive}
                    aria-label="Toggle connection enabled"
                  />
                </div>
              ) : (
                <div className="flex items-center gap-2 py-2.5 text-[11px] text-text-tertiary">
                  <Lock className="h-3 w-3 shrink-0" />
                  {readOnlyReason(key)}
                </div>
              )}
            </div>

            <DialogFooter className="flex-col-reverse gap-2 sm:flex-row sm:items-center sm:justify-between">
              <Button asChild variant="ghost" size="sm">
                <Link to="/keys/$keyId" params={{ keyId: key.id }}>
                  Open full settings
                  <ExternalLink />
                </Link>
              </Button>
              {modifiable &&
                (confirmingRevoke ? (
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] text-text-tertiary">
                      Revoke this connection?
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setConfirmingRevoke(false)}
                    >
                      Cancel
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      isLoading={deleteKey.isPending}
                      onClick={revoke}
                    >
                      Revoke
                    </Button>
                  </div>
                ) : (
                  <Button
                    variant="outline"
                    size="sm"
                    className="text-destructive hover:text-destructive"
                    onClick={() => setConfirmingRevoke(true)}
                  >
                    Revoke
                  </Button>
                ))}
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
