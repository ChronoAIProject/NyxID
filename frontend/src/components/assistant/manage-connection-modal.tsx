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
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  useDeleteKey,
  useKey,
  useUpdateExternalApiKey,
  useUpdateKey,
} from "@/hooks/use-keys";
import { ApiError } from "@/lib/api-client";
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

/**
 * Whether a pasted secret can replace this credential. OAuth and node-managed
 * credentials aren't user-held strings, and a connection still awaiting its
 * first auth has nothing to rotate — the same gate `ApiKeySection` applies on
 * the Studio page (key-detail.tsx:453).
 */
function canReplaceCredential(key: KeyInfo): boolean {
  return (
    Boolean(key.api_key_id) &&
    key.credential_type !== "oauth2" &&
    key.credential_type !== "node_managed" &&
    key.status !== "pending_auth"
  );
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

/** Inline credential replacement — the one rotation path a plugin user needs,
 *  kept in this modal so managing a connection never leaves it. */
function ReplaceCredential({ apiKeyId }: { readonly apiKeyId: string }) {
  const [open, setOpen] = useState(false);
  const [credential, setCredential] = useState("");
  const updateApiKey = useUpdateExternalApiKey();

  function submit() {
    const next = credential.trim();
    if (!next) return;
    updateApiKey.mutate(
      { keyId: apiKeyId, credential: next },
      {
        onSuccess: () => {
          toast.success("Credential replaced");
          setCredential("");
          setOpen(false);
        },
        onError: (error) =>
          toast.error(
            error instanceof ApiError
              ? error.message
              : "Could not replace the credential.",
          ),
      },
    );
  }

  if (!open) {
    return (
      <div className="flex items-center justify-between gap-4 py-2.5">
        <div>
          <p className="text-[12px] font-medium text-foreground">Credential</p>
          <p className="text-[11px] text-text-tertiary">
            Paste a new secret to replace the stored one.
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => setOpen(true)}
        >
          Replace
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-2 py-2.5">
      <p className="text-[12px] font-medium text-foreground">
        Replace credential
      </p>
      <Input
        type="password"
        autoFocus
        value={credential}
        onChange={(event) => setCredential(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            submit();
          }
        }}
        placeholder="New credential"
        aria-label="New credential"
      />
      <div className="flex justify-end gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => {
            setCredential("");
            setOpen(false);
          }}
        >
          Cancel
        </Button>
        <Button
          type="button"
          variant="primary"
          size="sm"
          disabled={!credential.trim()}
          isLoading={updateApiKey.isPending}
          onClick={submit}
        >
          Save
        </Button>
      </div>
    </div>
  );
}

/**
 * One connection's full management surface. Rendered once per credential, so a
 * service with several connections shows them stacked in the same modal rather
 * than making the user pick one first.
 */
function ConnectionPanel({
  keyId,
  showLabel,
  onRevoked,
}: {
  readonly keyId: string;
  /** Multi-connection cards head each panel with its credential label. */
  readonly showLabel: boolean;
  readonly onRevoked: () => void;
}) {
  const { data: key, isLoading, error, refetch } = useKey(keyId);
  const updateKey = useUpdateKey();
  const deleteKey = useDeleteKey();
  const [confirmingRevoke, setConfirmingRevoke] = useState(false);

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
        onRevoked();
      },
      onError: () => toast.error("Could not revoke the connection."),
    });
  }

  if (isLoading) {
    return (
      <div className="flex h-40 items-center justify-center">
        <Loader2 className="h-4 w-4 animate-spin text-text-tertiary" />
      </div>
    );
  }

  if (error || !key) {
    return (
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
    );
  }

  const grantedScopes = key.granted_scopes ?? null;
  const modifiable = canModifyKey(key);

  return (
    <section>
      {showLabel && (
        <div className="flex items-center justify-between gap-2 pb-1">
          <p className="min-w-0 truncate text-[12px] font-medium text-foreground">
            {key.label}
          </p>
          <Badge variant={statusVariant(key.status)}>
            {key.status.replaceAll("_", " ")}
          </Badge>
        </div>
      )}

      <div className="divide-y divide-border/60">
        {/* Multi-connection panels carry status beside their label; a lone
            connection has no label row, so it reads as a field instead. */}
        {!showLabel && (
          <DetailRow label="Status">
            <Badge variant={statusVariant(key.status)}>
              {key.status.replaceAll("_", " ")}
            </Badge>
          </DetailRow>
        )}
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
          <>
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
            {canReplaceCredential(key) && (
              <ReplaceCredential apiKeyId={key.api_key_id as string} />
            )}
          </>
        ) : (
          <div className="flex items-center gap-2 py-2.5 text-[11px] text-text-tertiary">
            <Lock className="h-3 w-3 shrink-0" />
            {readOnlyReason(key)}
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2 pt-2.5">
        <Button asChild variant="ghost" size="sm">
          <Link to="/keys/$keyId" params={{ keyId: key.id }}>
            Advanced settings
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
      </div>
    </section>
  );
}

/**
 * Connection management for the assistant Plugins surface. Everything a plugin
 * user needs lives in this one modal — including every credential behind a
 * service, stacked rather than staged behind a picker. Deep service config
 * (endpoint, routing, headers, node setup) stays on the Studio key page, one
 * click away per connection.
 */
export function ManageConnectionModal({
  keyIds,
  serviceName,
  iconSlug,
  onClose,
}: {
  readonly keyIds: readonly string[];
  readonly serviceName: string;
  readonly iconSlug?: string | null;
  readonly onClose: () => void;
}) {
  const multiple = keyIds.length > 1;

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2.5">
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-muted text-[12px] font-semibold text-muted-foreground">
              {iconSlug ? (
                <ServiceIcon slug={iconSlug} size="md" />
              ) : (
                connectorInitial(serviceName)
              )}
            </span>
            <span className="min-w-0 truncate">{serviceName}</span>
            {multiple && (
              <Badge variant="secondary" className="ml-1">
                {keyIds.length} connections
              </Badge>
            )}
          </DialogTitle>
          <DialogDescription className="sr-only">
            Manage the credentials this service is connected with.
          </DialogDescription>
        </DialogHeader>

        <div className="divide-y divide-border">
          {keyIds.map((keyId) => (
            <div key={keyId} className="py-4 first:pt-0 last:pb-0">
              <ConnectionPanel
                keyId={keyId}
                showLabel={multiple}
                // Revoking the last connection empties the modal, so close it;
                // with others left, the card stays open on what remains.
                onRevoked={multiple ? () => undefined : onClose}
              />
            </div>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
