import { Label } from "@/components/ui/label";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Copy, ExternalLink, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { ServiceIcon } from "@/components/service-icon";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedSaveBar,
} from "@/components/ui/grouped-list";
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
  return (
    !readOnly && !key.auto_connected && key.credential_binding !== "platform"
  );
}

function readOnlyReason(key: KeyInfo): string {
  if (key.auto_connected || key.credential_binding === "platform") {
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
  const providerCanAuthorize =
    key.credential_type === "oauth2" ||
    catalogEntry?.provider_type === "oauth2" ||
    catalogEntry?.provider_type === "device_code";
  return (
    canModifyKey(key) &&
    providerCanAuthorize &&
    (key.credential_missing || isReconnectableStatus(key.status))
  );
}

function DetailRow({
  label,
  children,
}: {
  readonly label: string;
  readonly children: React.ReactNode;
}) {
  return <GroupedRow title={label} accessory={children} />;
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

  if (!open)
    return (
      <GroupedList footer="Paste a new secret to replace the stored one.">
        <GroupedRow asChild interactive>
          <button type="button" onClick={() => setOpen(true)}>
            Replace
          </button>
        </GroupedRow>
      </GroupedList>
    );
  return (
    <div>
      <GroupedList title="Replace credential">
        <GroupedRow>
          <Label className="w-full text-text-tertiary">
            New credential
            <GroupedInput
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
            />
          </Label>
        </GroupedRow>
        {!credential && (
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => setOpen(false)}>Cancel</button>
          </GroupedRow>
        )}
      </GroupedList>
      {credential && (
        <GroupedSaveBar>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={updateApiKey.isPending}
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
        </GroupedSaveBar>
      )}
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
            : "Removed from NyxID. Upstream revocation is not confirmed.",
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
        <p className="text-14 text-muted-foreground">
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
      <div className="space-y-6">
        <GroupedList
          title={showLabel ? key.label : undefined}
          footer={
            <>
              {key.credential_missing ? (
                <p className="text-warning">
                  The stored credential is missing. Reconnect or delete this
                  service.
                </p>
              ) : isProblemStatus(key.status) ? (
                <>
                  <p
                    className={
                      statusMeta.variant === "destructive"
                        ? "workspace-status-destructive"
                        : "text-warning"
                    }
                  >
                    {statusMeta.tooltip}
                  </p>
                  {key.error_message ? (
                    <p className="break-words">{key.error_message}</p>
                  ) : null}
                </>
              ) : null}
              {modifiable ? (
                <p>
                  Disable to stop the assistant using this connection. Your
                  credential is kept — you can enable it again any time.
                </p>
              ) : (
                <p>{readOnlyReason(key)}</p>
              )}
            </>
          }
        >
          <DetailRow label="Status">{statusMeta.label}</DetailRow>
          <DetailRow label="Credential">
            {key.credential_missing
              ? "Missing - reconnect or delete"
              : key.credential_type.replaceAll("_", " ")}
          </DetailRow>
          {grantedScopes && grantedScopes.length > 0 ? (
            <DetailRow label="Scopes">
              <span className="break-words">{grantedScopes.join(" · ")}</span>
            </DetailRow>
          ) : null}
          {key.proxy_url ? (
            <DetailRow label="Proxy URL">
              <span className="inline-flex max-w-full items-start gap-2">
                <span className="min-w-0 break-all text-right text-13">
                  {key.proxy_url}
                </span>
                <button
                  type="button"
                  data-icon-button=""
                  aria-label="Copy proxy URL"
                  onClick={() => {
                    void navigator.clipboard?.writeText(key.proxy_url ?? "");
                    toast.success("Proxy URL copied");
                  }}
                  className="text-text-tertiary hover:text-foreground"
                >
                  <Copy className="size-[1em] shrink-0" />
                </button>
              </span>
            </DetailRow>
          ) : null}
          <DetailRow label="Last used">
            {key.last_used_at ? formatDate(key.last_used_at) : "Never"}
          </DetailRow>
          {modifiable ? (
            <GroupedRow
              title="Enabled"
              accessory={
                <Switch
                  checked={key.is_active}
                  disabled={
                    updateKey.isPending ||
                    (Boolean(key.credential_missing) && !key.is_active)
                  }
                  onCheckedChange={toggleActive}
                  aria-label="Toggle connection enabled"
                />
              }
            />
          ) : null}
          {reconnectable ? (
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => setReconnectOpen(true)}>
                {key.credential_missing
                  ? "Reconnect"
                  : reconnectLabel(key.status)}
              </button>
            </GroupedRow>
          ) : null}
        </GroupedList>
        {modifiable && canReplaceCredential(key) && !key.credential_missing ? (
          <ReplaceCredential apiKeyId={key.api_key_id as string} />
        ) : null}
        <GroupedList>
          <GroupedRow asChild interactive>
            <Link to="/keys/$keyId" params={{ keyId: key.id }}>
              <span className="flex-1">Advanced settings</span>
              <ExternalLink
                aria-hidden
                className="size-[1.125em] text-text-tertiary"
              />
            </Link>
          </GroupedRow>
          {modifiable ? (
            <GroupedRow asChild interactive className="text-destructive">
              <button
                type="button"
                onClick={() => setConfirmingRevoke(true)}
              >
                Delete
              </button>
            </GroupedRow>
          ) : null}
        </GroupedList>
      </div>
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
  iconUrl,
  onClose,
}: {
  readonly keyIds: readonly string[];
  readonly serviceName: string;
  readonly iconSlug?: string | null;
  /** Uploaded icon; preferred over the slug glyph when present. */
  readonly iconUrl?: string | null;
  readonly onClose: () => void;
}) {
  const multiple = keyIds.length > 1;

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="bg-background md:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2.5">
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-muted type-body-strong text-muted-foreground">
              {iconSlug || iconUrl ? (
                <ServiceIcon slug={iconSlug} iconUrl={iconUrl} size="md" />
              ) : (
                connectorInitial(serviceName)
              )}
            </span>
            <span className="min-w-0 truncate">{serviceName}</span>
            {multiple && (
              <span className="type-secondary text-text-tertiary">
                {keyIds.length} connections
              </span>
            )}
          </DialogTitle>
          <DialogDescription className="sr-only">
            Manage the credentials this service is connected with.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-6 pb-5 md:pb-0">
          {keyIds.map((keyId) => (
            <div key={keyId} className="space-y-6">
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
