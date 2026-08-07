import { useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  AlertTriangle,
  Copy,
  ExternalLink,
  Loader2,
  Lock,
  RefreshCw,
} from "lucide-react";
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
  useCatalogEntry,
  useKey,
  useUpdateExternalApiKey,
  useUpdateKey,
} from "@/hooks/use-keys";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { connectorInitial } from "@/lib/assistant/plugins";
import {
  credentialStatusMeta,
  isProblemStatus,
  isReconnectableStatus,
  reconnectLabel,
} from "@/lib/credential-status";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { GrantCascadeDialog } from "@/components/shared/grant-cascade-dialog";
import { DeleteConnectionDialog } from "@/components/shared/delete-connection-dialog";
import {
  parseGrantCascadeDetails,
  type GrantCascadeDetails,
} from "@/schemas/oauth-revocation";

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

/**
 * Whether a fresh authorization can repair this connection — the same gate
 * the Studio page applies before offering its Reconnect button
 * (key-detail.tsx `canReconnect`). Only OAuth-shaped credentials can be
 * repaired this way; a pasted API key is fixed by replacing the secret.
 */
function canReconnect(
  key: KeyInfo,
  catalogEntry: CatalogEntry | undefined,
): boolean {
  return (
    canModifyKey(key) &&
    isReconnectableStatus(key.status) &&
    (key.credential_type === "oauth2" ||
      catalogEntry?.provider_type === "oauth2" ||
      catalogEntry?.provider_type === "device_code")
  );
}

/**
 * Explains a connection that isn't working, and offers the way out.
 *
 * Without this the modal showed a lone red `failed` pill: a status the user
 * has no way to interpret (it covers both "the authorization never came
 * back" and "the provider rejected our stored grant"), no sight of the
 * `error_message` the backend already recorded on the row, and no action
 * other than leaving for the Studio page. `tooltip` from the status registry
 * is the general meaning; `errorMessage` is the provider's specific reason,
 * so both are shown when both exist.
 */
function ConnectionStatusNotice({
  status,
  errorMessage,
  onReconnect,
}: {
  readonly status: string;
  readonly errorMessage: string | null;
  /** Omitted when this status isn't repairable by re-authorizing. */
  readonly onReconnect?: () => void;
}) {
  const meta = credentialStatusMeta(status);
  const destructive = meta.variant === "destructive";

  return (
    <div
      className={
        destructive
          ? "flex items-start gap-3 rounded-xl border border-destructive/15 bg-destructive/[0.04] px-4 py-3"
          : "flex items-start gap-3 rounded-xl border border-warning/15 bg-warning/[0.04] px-4 py-3"
      }
    >
      <span
        className={
          destructive
            ? "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive"
            : "flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-warning/10 text-warning"
        }
      >
        <AlertTriangle className="h-4 w-4" />
      </span>
      <div className="min-w-0 flex-1 space-y-1.5">
        <p
          className={
            destructive
              ? "text-[12px] text-destructive"
              : "text-[12px] text-warning"
          }
        >
          {meta.tooltip}
        </p>
        {errorMessage && (
          <p className="break-words font-mono text-[11px] text-muted-foreground">
            {errorMessage}
          </p>
        )}
        {onReconnect && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="mt-0.5"
            onClick={onReconnect}
          >
            <RefreshCw />
            {reconnectLabel(status)}
          </Button>
        )}
      </div>
    </div>
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
  const { data: catalogEntry } = useCatalogEntry(
    key?.catalog_service_slug ?? null,
  );
  const updateKey = useUpdateKey();
  const deleteKey = useDeleteKey();
  const [confirmingRevoke, setConfirmingRevoke] = useState(false);
  const [reconnectOpen, setReconnectOpen] = useState(false);
  const [cascadeDetails, setCascadeDetails] =
    useState<GrantCascadeDetails | null>(null);

  function toggleActive(nextActive: boolean) {
    if (!key) return;
    updateKey.mutate(
      { keyId: key.id, is_active: nextActive },
      {
        onSuccess: () =>
          toast.success(
            nextActive ? "Connection enabled" : "Connection disabled",
          ),
        onError: () => toast.error("Could not update the connection."),
      },
    );
  }

  function revoke(
    input:
      | string
      | { keyId: string; cascadeGrant?: boolean; grantScope?: "token" },
  ) {
    if (!key) return;
    deleteKey.mutate(input, {
      onSuccess: (response) => {
        toast.success(
          response?.upstream_revocation_scheduled === true
            ? "Removed from NyxID. Upstream revocation scheduled."
            : "Removed from NyxID. Upstream access remains active.",
        );
        setCascadeDetails(null);
        // A service with other connections keeps the modal open on what's
        // left, so the confirmation has to be disarmed explicitly — otherwise
        // it lingers over a connection that no longer exists.
        setConfirmingRevoke(false);
        onRevoked();
      },
      onError: (mutationError) => {
        const details =
          mutationError instanceof ApiError
            ? parseGrantCascadeDetails(mutationError.errorResponse)
            : null;
        if (details) {
          setCascadeDetails(details);
          setConfirmingRevoke(false);
          return;
        }
        toast.error("Could not delete the connection.");
      },
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
  const statusMeta = credentialStatusMeta(key.status);
  const revokesGrant =
    key.revocation?.revokes_grant ??
    catalogEntry?.revocation?.revokes_grant ??
    false;
  const providerName =
    catalogEntry?.name ?? key.catalog_service_name ?? "provider";
  const reconnectable = canReconnect(key, catalogEntry);

  return (
    <>
      <section>
        {showLabel && (
          <div className="flex items-center justify-between gap-2 pb-1">
            <p className="min-w-0 truncate text-[12px] font-medium text-foreground">
              {key.label}
            </p>
            <Badge variant={statusMeta.variant}>{statusMeta.label}</Badge>
          </div>
        )}

        {/* A broken connection is the reason this modal gets opened, so the
            explanation leads rather than sitting under the detail rows. */}
        {isProblemStatus(key.status) && (
          <div className="pb-3">
            <ConnectionStatusNotice
              status={key.status}
              errorMessage={key.error_message}
              onReconnect={
                reconnectable ? () => setReconnectOpen(true) : undefined
              }
            />
          </div>
        )}

        <div className="divide-y divide-border/60">
          {/* Multi-connection panels carry status beside their label; a lone
            connection has no label row, so it reads as a field instead. */}
          {!showLabel && (
            <DetailRow label="Status">
              <Badge variant={statusMeta.variant}>{statusMeta.label}</Badge>
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
                    Disable to stop the assistant using this connection. Your
                    credential is kept — you can enable it again any time.
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
          {modifiable && (
            <Button
              variant="outline"
              size="sm"
              className="text-destructive hover:text-destructive"
              onClick={() => setConfirmingRevoke(true)}
            >
              Delete
            </Button>
          )}
        </div>
      </section>
      {/* Confirmation is a modal, not an inline swap in the footer: the
          grant-revoking copy is a full sentence about de-authorizing an
          upstream account, and rendered in-card it read as an error the app
          had raised rather than a step the user had just triggered. */}
      {confirmingRevoke && (
        <DeleteConnectionDialog
          providerName={providerName}
          connectionLabel={showLabel ? key.label : null}
          revokesGrant={revokesGrant}
          isPending={deleteKey.isPending}
          onConfirm={() => revoke(key.id)}
          onCancel={() => setConfirmingRevoke(false)}
        />
      )}
      {cascadeDetails && (
        <GrantCascadeDialog
          details={cascadeDetails}
          isPending={deleteKey.isPending}
          onCascade={() => revoke({ keyId: key.id, cascadeGrant: true })}
          onRemoveOnly={() => revoke({ keyId: key.id, grantScope: "token" })}
          onCancel={() => setCascadeDetails(null)}
        />
      )}
      {/* The same dialog the Studio page and assistant connect cards use for
          re-authorization, so every reconnect entry point runs one flow.
          Mounted only while open: a service with several connections renders
          one panel each, and an idle wizard per panel would run its hooks for
          a flow the user never started. */}
      {reconnectOpen && (
        <AddKeyDialog open onOpenChange={setReconnectOpen} reconnectKey={key} />
      )}
    </>
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
