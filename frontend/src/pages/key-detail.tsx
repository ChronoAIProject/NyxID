import { useEffect, useMemo, useState } from "react";
import {
  Link,
  useParams,
  useNavigate,
  useSearch,
} from "@tanstack/react-router";
import {
  useKey,
  useDeleteKey,
  useUpdateKey,
  useUpdateEndpoint,
  useUpdateExternalApiKey,
  useUpdateUserService,
  useCatalogEntry,
} from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import { DefaultHeadersEditor } from "@/components/shared/default-headers-editor";
import { WsFrameInjectionsEditor } from "@/components/shared/ws-frame-injections-editor";
import { defaultRequestHeaderListSchema } from "@/schemas/default-request-headers";
import type { DefaultRequestHeader } from "@/schemas/default-request-headers";
import {
  wsFrameInjectionsSchema,
  type WsFrameInjection,
} from "@/schemas/services";
import { ApiError } from "@/lib/api-client";
import { copyToClipboard } from "@/lib/utils";
import { ErrorBanner } from "@/components/shared/error-banner";
import { PageHeader } from "@/components/shared/page-header";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { SshServiceInstructions } from "@/components/dashboard/ssh-service-instructions";
import { RoutingSection } from "@/components/dashboard/routing-section";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Globe,
  KeyRound,
  Server,
  Pencil,
  Trash2,
  RefreshCw,
  Check,
  X,
  Terminal,
  Copy,
  Shield,
  ShieldCheck,
  Code,
  ExternalLink,
  FileJson,
  Power,
} from "lucide-react";
import { toast } from "sonner";
import type { SshServiceConfig } from "@/types/api";
import type { CatalogEntry } from "@/types/keys";

function statusVariant(
  status: string,
): "success" | "secondary" | "destructive" {
  switch (status) {
    case "active":
      return "success";
    case "expired":
      return "secondary";
    case "revoked":
    case "failed":
    case "refresh_failed":
      return "destructive";
    default:
      return "secondary";
  }
}

function titleCase(s: string): string {
  return s
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

const RECONNECTABLE_STATUSES = new Set([
  "pending_auth",
  "refresh_failed",
  "failed",
]);

function reconnectLabel(status: string): string {
  return status === "pending_auth"
    ? "Continue authentication"
    : "Reconnect";
}

/**
 * Renders the Lark / Feishu developer-console permission deep link the
 * backend attaches to `permission_setup_url`. Shown only for AI Services
 * keys backed by the `api-lark`, `api-lark-bot`, `api-feishu`, or
 * `api-feishu-bot` catalog entries when the credential carries a usable
 * `app_id`. The deep link lands the user on the matching app's
 * "Permissions & Scopes" page with the catalog's required scopes
 * pre-selected, ready for "Bulk Enable".
 */
function LarkPermissionSetupCard({
  url,
  scopes,
}: {
  readonly url: string;
  readonly scopes: readonly string[];
}) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <ShieldCheck className="h-4 w-4 text-text-tertiary" />
            <CardTitle className="text-[15px]">Configure Permissions</CardTitle>
          </div>
          <Button variant="primary" asChild>
            <a href={url} target="_blank" rel="noopener noreferrer">
              Open Permissions Page
              <ButtonIcon variant="primary"><ExternalLink className="h-3 w-3" /></ButtonIcon>
            </a>
          </Button>
        </div>
        <CardDescription>
          Open this link to grant the scopes this service needs in the Lark /
          Feishu developer console. The required scopes are pre-selected —
          confirm and bulk-enable them so NyxID can call the API on your
          behalf.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {scopes.length > 0 && (
          <div>
            <p className="text-xs font-medium text-text-tertiary uppercase tracking-wide">
              Scopes pre-selected
            </p>
            <ul className="mt-2 flex flex-wrap gap-2">
              {scopes.map((scope) => (
                <li key={scope}>
                  <Badge variant="secondary" className="text-xs">
                    {scope}
                  </Badge>
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function EndpointSection({
  endpointUrl,
  endpointId,
  nodeRouted,
  readOnly = false,
}: {
  readonly endpointUrl: string;
  readonly endpointId: string;
  readonly nodeRouted: boolean;
  readonly readOnly?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [url, setUrl] = useState(endpointUrl);
  const updateEndpoint = useUpdateEndpoint();

  function handleSave() {
    if (!url.trim()) return;
    updateEndpoint.mutate(
      { endpointId, url: url.trim() },
      {
        onSuccess: () => {
          toast.success("Endpoint updated");
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update endpoint";
          toast.error(message);
        },
      },
    );
  }

  function handleCancel() {
    setUrl(endpointUrl);
    setEditing(false);
  }

  const isEmpty = !endpointUrl;

  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Globe className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">Endpoint</CardTitle>
        </div>
        <CardDescription>Target URL for proxied requests</CardDescription>
      </CardHeader>
      <CardContent>
        {editing ? (
          <div className="flex items-center gap-2">
            <Input
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://api.example.com/v1"
              className="flex-1 text-[12px]"
            />
            <Button size="icon" variant="ghost" onClick={handleCancel}>
              <X className="h-4 w-4" />
            </Button>
            <Button
              size="icon"
              variant="ghost"
              onClick={handleSave}
              disabled={updateEndpoint.isPending}
            >
              <Check className="h-4 w-4" />
            </Button>
          </div>
        ) : isEmpty && nodeRouted ? (
          <div className="flex items-center justify-between gap-2">
            <Badge variant="secondary">Resolved by node agent</Badge>
            {!readOnly && (
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6 shrink-0"
                onClick={() => setEditing(true)}
              >
                <Pencil className="h-3 w-3" />
              </Button>
            )}
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            <code className="truncate rounded bg-muted px-2 py-1 font-mono text-[12px]">
              {endpointUrl}
            </code>
            {!readOnly && (
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6 shrink-0"
                onClick={() => setEditing(true)}
              >
                <Pencil className="h-3 w-3" />
              </Button>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function OpenApiSpecSection({
  endpointId,
  specUrl,
  readOnly = false,
}: {
  readonly endpointId: string;
  readonly specUrl: string | null;
  readonly readOnly?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  // Seeded only when entering edit mode (handleEdit), not from specUrl
  // changes, to avoid a setState-in-effect loop that lint flags.
  const [draft, setDraft] = useState("");
  const updateEndpoint = useUpdateEndpoint();

  function handleEdit() {
    setDraft(specUrl ?? "");
    setEditing(true);
  }

  function handleSave() {
    const trimmed = draft.trim();
    updateEndpoint.mutate(
      { endpointId, openapi_spec_url: trimmed },
      {
        onSuccess: () => {
          toast.success(
            trimmed ? "OpenAPI spec URL saved" : "OpenAPI spec URL cleared",
          );
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to update OpenAPI spec URL";
          toast.error(message);
        },
      },
    );
  }

  function handleCancel() {
    setEditing(false);
  }

  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <FileJson className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">OpenAPI Spec</CardTitle>
        </div>
        <CardDescription>
          Optional — lets AI agents discover concrete API operations instead of
          falling back to a single generic proxy tool.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {editing ? (
          <div className="flex items-center gap-2">
            <Input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="https://api.example.com/openapi.json"
              className="flex-1 text-[12px]"
              type="url"
            />
            <Button size="icon" variant="ghost" onClick={handleCancel}>
              <X className="h-4 w-4" />
            </Button>
            <Button
              size="icon"
              variant="ghost"
              onClick={handleSave}
              disabled={updateEndpoint.isPending}
            >
              <Check className="h-4 w-4" />
            </Button>
          </div>
        ) : specUrl ? (
          <div className="flex items-center justify-between gap-2">
            <code className="truncate rounded bg-muted px-2 py-1 font-mono text-[12px]">
              {specUrl}
            </code>
            {!readOnly && (
              <Button size="icon" variant="ghost" className="h-6 w-6 shrink-0" onClick={handleEdit}>
                <Pencil className="h-3 w-3" />
              </Button>
            )}
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            <Badge variant="secondary">Not set</Badge>
            {!readOnly && (
              <Button size="icon" variant="ghost" className="h-6 w-6 shrink-0" onClick={handleEdit}>
                <Pencil className="h-3 w-3" />
              </Button>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function ApiKeySection({
  apiKeyId,
  credentialType,
  status,
  expiresAt,
  lastUsedAt,
  errorMessage,
  readOnly = false,
}: {
  readonly apiKeyId: string;
  readonly credentialType: string;
  readonly status: string;
  readonly expiresAt: string | null;
  readonly lastUsedAt: string | null;
  readonly errorMessage: string | null;
  readonly readOnly?: boolean;
}) {
  const [rotating, setRotating] = useState(false);
  const [newCredential, setNewCredential] = useState("");
  const updateApiKey = useUpdateExternalApiKey();

  function handleRotate() {
    if (!newCredential.trim()) return;
    updateApiKey.mutate(
      { keyId: apiKeyId, credential: newCredential.trim() },
      {
        onSuccess: () => {
          toast.success("Credential rotated");
          setRotating(false);
          setNewCredential("");
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to rotate credential";
          toast.error(message);
        },
      },
    );
  }

  // Show the rotate button in the header only when it would have been
  // shown at the bottom: not readOnly, not node_managed, not pending_auth,
  // not oauth2, and not currently in rotating mode.
  const showRotateInHeader =
    !readOnly &&
    credentialType !== "node_managed" &&
    status !== "pending_auth" &&
    credentialType !== "oauth2" &&
    !rotating;

  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <KeyRound className="h-4 w-4 text-primary" />
            <CardTitle className="text-[15px]">API Key</CardTitle>
          </div>
          {showRotateInHeader && (
            <Button variant="outline" className="text-text-tertiary hover:text-muted-foreground" onClick={() => setRotating(true)}>
              <ButtonIcon><RefreshCw className="h-3 w-3" /></ButtonIcon>
              Rotate Credentials
            </Button>
          )}
        </div>
        <CardDescription>Authentication credential</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-3">
          <Badge variant={statusVariant(status)}>{titleCase(status)}</Badge>
          <span className="text-xs text-muted-foreground">
            Type: {credentialType}
          </span>
        </div>

        {errorMessage && (
          <p className="text-xs text-destructive">{errorMessage}</p>
        )}

        {expiresAt && (
          <p className="text-xs text-muted-foreground">
            Expires: {new Date(expiresAt).toLocaleString()}
          </p>
        )}

        {lastUsedAt && (
          <p className="text-xs text-muted-foreground">
            Last used: {new Date(lastUsedAt).toLocaleString()}
          </p>
        )}

        {readOnly ? (
          <p className="text-xs text-muted-foreground">
            This credential is shared from an org. Ask an admin of the owning
            org to rotate or replace it.
          </p>
        ) : credentialType === "node_managed" ? (
          <p className="text-xs text-muted-foreground">
            This credential is managed on the node agent. Update it on the node
            instead of storing it in NyxID.
          </p>
        ) : status === "pending_auth" ? (
          <p className="text-xs text-muted-foreground">
            Complete the required credential setup to activate this service.
          </p>
        ) : credentialType === "oauth2" ? (
          <p className="text-xs text-muted-foreground">
            This credential is managed through the provider connection flow.
          </p>
        ) : rotating ? (
          <div className="space-y-2">
            <Input
              type="password"
              value={newCredential}
              onChange={(e) => setNewCredential(e.target.value)}
              placeholder="Enter new credential"
            />
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                onClick={() => {
                  setRotating(false);
                  setNewCredential("");
                }}
              >
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={handleRotate}
                disabled={updateApiKey.isPending || !newCredential.trim()}
              >
                Save
              </Button>
            </div>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

function ServiceSection({
  slug,
  authMethod,
  authKeyName,
  isActive,
  credentialStatus,
  hasCredential,
  serviceId,
  customUserAgent,
  nodeId,
  nodeStatus,
  readOnly = false,
}: {
  readonly slug: string;
  readonly authMethod: string;
  readonly authKeyName: string;
  readonly isActive: boolean;
  /** API key status ("active" | "pending_auth" | "expired" | "revoked" |
   *  "failed" | "refresh_failed"). When the service has no credential (e.g. auto-connected
   *  downstreams) this is ignored. */
  readonly credentialStatus: string;
  /** Whether this service has an associated credential. Services without a
   *  credential (e.g. auto-connected or no-auth downstreams) skip the
   *  credential-readiness check entirely. */
  readonly hasCredential: boolean;
  readonly serviceId: string;
  readonly customUserAgent?: string | null;
  readonly nodeId?: string | null;
  readonly nodeStatus?: string | null;
  readonly readOnly?: boolean;
}) {
  const updateService = useUpdateUserService();
  const [editingUa, setEditingUa] = useState(false);
  const [uaDraft, setUaDraft] = useState(customUserAgent ?? "");

  const isNodeBound = nodeId !== undefined && nodeId !== null && nodeId !== "";

  let badgeVariant: "success" | "secondary" | "destructive" = "secondary";
  let badgeLabel = "Inactive";
  let credentialBlocked = false;
  let nodeWarning: string | null = null;
  let nodeHint: string | null = null;

  if (!isActive) {
    badgeVariant = "secondary";
    badgeLabel = "Inactive";
  } else if (isNodeBound) {
    switch (nodeStatus) {
      case "unknown":
        badgeVariant = "destructive";
        badgeLabel = "Node Deleted";
        nodeWarning = "Bound node was not found on the server. The binding is broken. Use Routing section to re-bind this service to a valid node.";
        break;
      case "inaccessible":
        badgeVariant = "secondary";
        badgeLabel = "Inaccessible";
        nodeHint = "Binding points at a node you do not have permission to introspect. The proxy connection may still function normally.";
        break;
      case "offline":
        badgeVariant = "destructive";
        badgeLabel = "Offline";
        nodeWarning = "Bound node is offline or has a stale heartbeat. Use Routing section to re-bind this service to an online node.";
        break;
      case "draining":
        badgeVariant = "secondary";
        badgeLabel = "Draining";
        nodeWarning = "Bound node is draining. Use Routing section to re-bind this service to an online node.";
        break;
      case "online":
      default:
        credentialBlocked = hasCredential && credentialStatus !== "" && credentialStatus !== "active";
        if (credentialBlocked) {
          badgeVariant = "secondary";
          badgeLabel = "Unavailable";
        } else {
          badgeVariant = "success";
          badgeLabel = "Active";
        }
        break;
    }
  } else {
    credentialBlocked = hasCredential && credentialStatus !== "" && credentialStatus !== "active";
    if (credentialBlocked) {
      badgeVariant = "secondary";
      badgeLabel = "Unavailable";
    } else {
      badgeVariant = "success";
      badgeLabel = "Active";
    }
  }

  function toggleActive() {
    updateService.mutate(
      { serviceId, is_active: !isActive },
      {
        onSuccess: () => {
          toast.success(isActive ? "Service deactivated" : "Service activated");
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update service";
          toast.error(message);
        },
      },
    );
  }

  function saveUserAgent() {
    updateService.mutate(
      { serviceId, custom_user_agent: uaDraft.trim() || "" },
      {
        onSuccess: () => {
          setEditingUa(false);
          toast.success(
            uaDraft.trim()
              ? "Custom User-Agent saved"
              : "Custom User-Agent cleared",
          );
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to update User-Agent";
          toast.error(message);
        },
      },
    );
  }

  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Server className="h-4 w-4 text-primary" />
            <CardTitle className="text-[15px]">Service</CardTitle>
          </div>
          {!readOnly && (
            isActive ? (
              <Button
                variant="destructive"
                onClick={toggleActive}
                disabled={updateService.isPending}
              >
                <ButtonIcon variant="destructive"><Power className="h-3 w-3" /></ButtonIcon>
                Deactivate
              </Button>
            ) : (
              <Button
                variant="primary"
                onClick={toggleActive}
                disabled={updateService.isPending}
              >
                <ButtonIcon variant="primary"><Power className="h-3 w-3" /></ButtonIcon>
                Activate
              </Button>
            )
          )}
        </div>
        <CardDescription>Proxy routing configuration</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-3">
          <code className="rounded bg-muted px-2 py-1 font-mono text-[12px]">
            /proxy/s/{slug}
          </code>
          <Badge variant={badgeVariant}>{badgeLabel}</Badge>
        </div>

        {credentialBlocked && (
          <p className="text-xs text-muted-foreground">
            The service record is enabled, but its credential is{" "}
            <span className="font-medium text-foreground">
              {credentialStatus}
            </span>
            . Real requests will fail until the credential is restored.
          </p>
        )}

        {nodeWarning && (
          <p className="text-xs text-destructive font-medium">
            {nodeWarning}
          </p>
        )}
        {nodeHint && (
          <p className="text-xs text-muted-foreground font-medium">
            {nodeHint}
          </p>
        )}

        <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground">
          <div>
            <span className="font-medium text-foreground">Auth method:</span>{" "}
            {authMethod}
          </div>
          <div>
            <span className="font-medium text-foreground">Auth key:</span>{" "}
            {authKeyName}
          </div>
        </div>

        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">User-Agent</Label>
          {editingUa ? (
            <div className="flex items-center gap-2">
              <Input
                value={uaDraft}
                onChange={(e) => setUaDraft(e.target.value)}
                placeholder="Passthrough (default)"
                className="h-8 text-xs"
                maxLength={256}
              />
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8"
                onClick={saveUserAgent}
                disabled={updateService.isPending}
              >
                <Check className="h-3.5 w-3.5" />
              </Button>
              <Button
                size="icon"
                variant="ghost"
                className="h-8 w-8"
                onClick={() => {
                  setEditingUa(false);
                  setUaDraft(customUserAgent ?? "");
                }}
              >
                <X className="h-3.5 w-3.5" />
              </Button>
            </div>
          ) : (
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs">
                {customUserAgent || (
                  <Badge variant="secondary">Passthrough (default)</Badge>
                )}
              </span>
              {!readOnly && (
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6 shrink-0"
                  onClick={() => {
                    setUaDraft(customUserAgent ?? "");
                    setEditingUa(true);
                  }}
                >
                  <Pencil className="h-3 w-3" />
                </Button>
              )}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

// `RoutingSection` lives in `components/dashboard/routing-section.tsx`
// so the admin `/services/$id` page can reuse the same editable widget
// (issue #416). Imported at the top of this file.

function NodeSetupHelper({
  slug,
  endpointUrl,
  authMethod,
  authKeyName,
  catalogServiceName,
}: {
  readonly slug: string;
  readonly endpointUrl: string;
  readonly authMethod: string;
  readonly authKeyName: string;
  readonly catalogServiceName: string | null;
}) {
  const urlFlag = endpointUrl ? ` \\\n  --url ${endpointUrl}` : "";

  let credentialFlags: string;
  switch (authMethod) {
    case "bearer":
      credentialFlags = ` \\\n  --header ${authKeyName} \\\n  --secret-format bearer`;
      break;
    case "header":
      credentialFlags = ` \\\n  --header ${authKeyName}`;
      break;
    case "query":
      credentialFlags = ` \\\n  --query-param ${authKeyName}`;
      break;
    case "basic":
      credentialFlags = ` \\\n  --header ${authKeyName} \\\n  --secret-format basic`;
      break;
    case "none":
      credentialFlags = "";
      break;
    default:
      credentialFlags = ` \\\n  --header ${authKeyName}`;
  }

  const setupCommand = `nyxid node credentials setup --service ${slug}`;
  const manualCommand = `nyxid node credentials add \\\n  --service ${slug}${urlFlag}${credentialFlags}`;

  function handleCopySetup() {
    void copyToClipboard(setupCommand).then(() => {
      toast.success("Command copied to clipboard");
    });
  }

  function handleCopyManual() {
    void copyToClipboard(manualCommand).then(() => {
      toast.success("Command copied to clipboard");
    });
  }

  return (
    <Card className="min-w-0 md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Terminal className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">Node Setup</CardTitle>
        </div>
        <CardDescription>
          Run this on your node to configure credentials
          {catalogServiceName ? ` for ${catalogServiceName}` : ""}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-[11px] font-medium text-muted-foreground">
          Recommended (auto-detects requirements):
        </p>
        <div className="relative">
          <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 min-h-[44px] font-mono text-xs leading-relaxed">
            {setupCommand}
          </pre>
          <Button
            size="icon"
            variant="ghost"
            className="absolute right-2 top-2 h-7 w-7"
            onClick={handleCopySetup}
          >
            <Copy className="h-3.5 w-3.5" />
          </Button>
        </div>
        <p className="text-[11px] font-medium text-muted-foreground">Manual:</p>
        <div className="relative">
          <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 min-h-[44px] font-mono text-xs leading-relaxed">
            {manualCommand}
          </pre>
          <Button
            size="icon"
            variant="ghost"
            className="absolute right-2 top-2 h-7 w-7"
            onClick={handleCopyManual}
          >
            <Copy className="h-3.5 w-3.5" />
          </Button>
        </div>
        <p className="text-[11px] text-muted-foreground">
          The agent will prompt for the secret value securely. After adding, the
          credential will be encrypted and stored locally on the node.
        </p>
      </CardContent>
    </Card>
  );
}

function SshConnectionSection({
  sshHost,
  sshPort,
  caPublicKey,
  principals,
  certTtlMinutes,
}: {
  readonly sshHost: string;
  readonly sshPort: number;
  readonly caPublicKey: string | null;
  readonly principals: readonly string[] | null;
  readonly certTtlMinutes: number | null;
}) {
  function handleCopyCa() {
    if (!caPublicKey) return;
    void copyToClipboard(caPublicKey).then(() => {
      toast.success("CA public key copied");
    });
  }

  return (
    <Card className="md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Shield className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">SSH Connection</CardTitle>
        </div>
        <CardDescription>
          SSH certificate authentication details
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-4 text-[12px]">
          <div>
            <span className="text-xs font-medium text-muted-foreground">
              Host
            </span>
            <p>
              {sshHost}:{sshPort}
            </p>
          </div>
          {certTtlMinutes !== null && (
            <div>
              <span className="text-xs font-medium text-muted-foreground">
                Certificate TTL
              </span>
              <p>{certTtlMinutes} minutes</p>
            </div>
          )}
        </div>

        {principals && principals.length > 0 && (
          <div>
            <span className="text-xs font-medium text-muted-foreground">
              Allowed Principals
            </span>
            <div className="mt-1 flex flex-wrap gap-1.5">
              {principals.map((p) => (
                <Badge key={p} variant="secondary" className="text-xs">
                  {p}
                </Badge>
              ))}
            </div>
          </div>
        )}

        {caPublicKey && (
          <div>
            <span className="text-xs font-medium text-muted-foreground">
              CA Public Key
            </span>
            <div className="relative mt-1">
              <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-xs leading-relaxed">
                {caPublicKey}
              </pre>
              <Button
                size="icon"
                variant="ghost"
                className="absolute right-2 top-2 h-7 w-7"
                onClick={handleCopyCa}
              >
                <Copy className="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
        )}

        <div className="rounded-lg border border-border bg-muted/50 p-3 space-y-2">
          <p className="text-xs font-medium">Target Machine Setup</p>
          <ol className="list-decimal list-inside space-y-1 text-xs text-muted-foreground">
            <li>
              Add the CA public key to{" "}
              <code className="rounded bg-background px-1">
                /etc/ssh/trusted-user-ca-keys.pem
              </code>
            </li>
            <li>
              Add{" "}
              <code className="rounded bg-background px-1">
                TrustedUserCAKeys /etc/ssh/trusted-user-ca-keys.pem
              </code>{" "}
              to{" "}
              <code className="rounded bg-background px-1">
                /etc/ssh/sshd_config
              </code>
            </li>
            <li>Restart sshd</li>
          </ol>
        </div>
      </CardContent>
    </Card>
  );
}

function endpointPathEndsWithV1(endpointUrl: string): boolean {
  try {
    const url = new URL(endpointUrl);
    return url.pathname.replace(/\/+$/, "").endsWith("/v1");
  } catch {
    const path = endpointUrl.split(/[?#]/, 1)[0] ?? endpointUrl;
    return /\/v1\/?$/.test(path);
  }
}

function chatCompletionsPath(endpointUrl: string): string {
  return endpointPathEndsWithV1(endpointUrl)
    ? "/chat/completions"
    : "/v1/chat/completions";
}

function serviceText(parts: readonly (string | null | undefined)[]): string {
  return parts.filter(Boolean).join(" ").toLowerCase();
}

function isBotOrWebhookService(text: string): boolean {
  return (
    ["telegram", "lark", "feishu", "discord"].some((term) =>
      text.includes(term),
    ) || /\b(bot|webhook)\b/.test(text)
  );
}

function isLlmService(text: string): boolean {
  return (
    /\bllm\b/.test(text) ||
    text.includes("llm-") ||
    ["openai", "deepseek", "anthropic", "gemini", "google"].some((term) =>
      text.includes(term),
    )
  );
}

function genericEndpointPathFor(metadataText: string): string | null {
  if (metadataText.includes("telegram")) {
    return "/getMe";
  }
  if (metadataText.includes("discord")) {
    return "/users/@me";
  }
  if (metadataText.includes("lark") || metadataText.includes("feishu")) {
    return "/open-apis/bot/v3/info";
  }
  return null;
}

function exampleModelForSlug(slugText: string): {
  readonly model: string;
  readonly needsProviderModelNote: boolean;
} {
  if (slugText.includes("openai")) {
    return { model: "gpt-4o", needsProviderModelNote: false };
  }
  if (slugText.includes("deepseek")) {
    return { model: "deepseek-chat", needsProviderModelNote: false };
  }
  if (slugText.includes("anthropic")) {
    return { model: "claude-sonnet-4-5", needsProviderModelNote: false };
  }
  if (slugText.includes("gemini") || slugText.includes("google")) {
    return { model: "gemini-2.0-flash", needsProviderModelNote: false };
  }
  return { model: "gpt-4o", needsProviderModelNote: true };
}

function buildCurlExample({
  method,
  url,
  authHeader,
  body,
}: {
  readonly method: "GET" | "POST";
  readonly url: string;
  readonly authHeader: string;
  readonly body: string | null;
}): string {
  const lines = [`curl ${method === "GET" ? "-X GET " : ""}${url} \\`];
  lines.push(`  -w "\\nHTTP=%{http_code}\\n" \\`);
  lines.push(`  -H "${authHeader}"`);

  if (body) {
    lines[lines.length - 1] = `${lines[lines.length - 1]} \\`;
    lines.push(`  -H "Content-Type: application/json" \\`);
    lines.push(`  -d '${body}'`);
  }

  return lines.join("\n");
}

function ApiUsageSection({
  slug,
  authMethod,
  endpointUrl,
  catalogServiceSlug,
  label,
  catalogEntry,
}: {
  readonly slug: string;
  readonly authMethod: string;
  readonly endpointUrl: string;
  readonly catalogServiceSlug: string | null;
  readonly label: string;
  readonly catalogEntry: CatalogEntry | undefined;
}) {
  const proxyUrl = `${window.location.origin}/api/v1/proxy/s/${slug}`;
  const catalogSlug = catalogServiceSlug ?? catalogEntry?.slug ?? slug;
  const slugText = serviceText([slug, catalogSlug]);
  const metadataText = serviceText([
    slug,
    catalogSlug,
    label,
    catalogEntry?.name,
    catalogEntry?.description,
    catalogEntry?.provider_type,
    catalogEntry?.service_type,
  ]);
  const showGenericEndpointExample = isBotOrWebhookService(metadataText);
  const llmDetected = isLlmService(metadataText);
  const llmExamplePath = chatCompletionsPath(endpointUrl);
  const genericExamplePath = showGenericEndpointExample
    ? genericEndpointPathFor(metadataText)
    : null;
  const examplePath = showGenericEndpointExample
    ? genericExamplePath
    : llmExamplePath;
  const exampleUrl = examplePath ? `${proxyUrl}${examplePath}` : null;
  const modelExample = llmDetected
    ? exampleModelForSlug(slugText)
    : { model: "gpt-4o", needsProviderModelNote: false };
  const requestBody =
    showGenericEndpointExample || !exampleUrl
      ? null
      : JSON.stringify({
          model: modelExample.model,
          messages: [{ role: "user", content: "hello" }],
        });
  const method = requestBody ? "POST" : "GET";

  const authNote =
    authMethod === "none"
      ? "This service requires no upstream credentials, but you still need to authenticate with NyxID."
      : "NyxID injects your stored credentials automatically when proxying.";

  const bearerTokenExample = exampleUrl
    ? buildCurlExample({
        method,
        url: exampleUrl,
        authHeader: "Authorization: Bearer <NYXID_ACCESS_TOKEN>",
        body: requestBody,
      })
    : null;

  const apiKeyExample = exampleUrl
    ? buildCurlExample({
        method,
        url: exampleUrl,
        authHeader: "X-API-Key: nyx_...",
        body: requestBody,
      })
    : null;

  function handleCopyUrl() {
    void copyToClipboard(proxyUrl).then(() => {
      toast.success("Proxy URL copied");
    });
  }

  function handleCopyExample(example: string) {
    void copyToClipboard(example).then(() => {
      toast.success("Example copied");
    });
  }

  return (
    <Card className="min-w-0 md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Code className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">API Usage</CardTitle>
        </div>
        <CardDescription>
          How to connect to this service through NyxID proxy
        </CardDescription>
      </CardHeader>
      <CardContent className="min-w-0 space-y-4">
        <div>
          <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">
            Base URL
          </p>
          <div className="relative">
            <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-[12px]">
              {proxyUrl}
            </pre>
            <Button
              size="icon"
              variant="ghost"
              className="absolute right-2 top-2 h-7 w-7"
              onClick={handleCopyUrl}
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
          </div>
          <p className="mt-1.5 text-[11px] text-muted-foreground">
            Append the downstream API path after this URL
            {examplePath ? (
              <>
                {" "}
                (e.g.{" "}
                <code className="rounded bg-background px-1">
                  {examplePath}
                </code>
                )
              </>
            ) : null}
            . {authNote}
          </p>
          {showGenericEndpointExample && (
            <p className="mt-1.5 text-[11px] text-muted-foreground">
              Run{" "}
              <code className="rounded bg-background px-1">
                nyxid catalog endpoints {catalogSlug}
              </code>{" "}
              to discover available endpoints for this service.
            </p>
          )}
        </div>

        <div>
          <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">
            Authentication
          </p>
          <div className="space-y-2 text-xs text-muted-foreground">
            <p>Authenticate with NyxID using one of these methods:</p>
            <ul className="list-disc list-inside space-y-1 pl-1">
              <li>
                <span className="font-medium text-foreground">API Key:</span>{" "}
                <code className="rounded bg-background px-1">
                  X-API-Key: nyx_...
                </code>{" "}
                header (create one in the{" "}
                <Link
                  to="/keys"
                  search={{ tab: "nyxid" }}
                  className="font-medium text-primary underline-offset-4 hover:underline"
                >
                  Agent Keys
                </Link>{" "}
                tab on AI Services)
              </li>
              <li>
                <span className="font-medium text-foreground">
                  Bearer Token:
                </span>{" "}
                <code className="rounded bg-background px-1">
                  Authorization: Bearer &lt;access_token&gt;
                </code>
              </li>
            </ul>
          </div>
        </div>

        {apiKeyExample && (
          <div>
            <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">
              Example (with API key)
            </p>
            {modelExample.needsProviderModelNote && requestBody && (
              <p className="mb-1.5 text-[11px] text-muted-foreground">
                Replace{" "}
                <code className="rounded bg-background px-1">gpt-4o</code>{" "}
                with your provider&apos;s model.
              </p>
            )}
            <div className="relative">
              <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-xs leading-relaxed">
                {apiKeyExample}
              </pre>
              <Button
                size="icon"
                variant="ghost"
                className="absolute right-2 top-2 h-7 w-7"
                onClick={() => handleCopyExample(apiKeyExample)}
              >
                <Copy className="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
        )}

        {bearerTokenExample && (
          <details className="rounded-lg border border-border bg-muted/20 p-3">
            <summary className="cursor-pointer text-xs font-medium text-muted-foreground">
              Advanced: Bearer token example
            </summary>
            <div className="mt-3 space-y-2">
              <p className="text-[11px] text-muted-foreground">
                Bearer auth is intended for self-hosted deployments or
                environments where you already have a NyxID access token. For
                most users, prefer the API Key example above.
              </p>
              <div className="relative">
                <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-xs leading-relaxed">
                  {bearerTokenExample}
                </pre>
                <Button
                  size="icon"
                  variant="ghost"
                  className="absolute right-2 top-2 h-7 w-7"
                  onClick={() => handleCopyExample(bearerTokenExample)}
                >
                  <Copy className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
          </details>
        )}
      </CardContent>
    </Card>
  );
}

function DeleteKeyDialog({
  open,
  onOpenChange,
  keyId,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly keyId: string;
}) {
  const navigate = useNavigate();
  const deleteKey = useDeleteKey();

  function handleDelete() {
    deleteKey.mutate(keyId, {
      onSuccess: () => {
        toast.success("Key deleted");
        void navigate({ to: "/keys", search: {} });
      },
      onError: (err) => {
        const message =
          err instanceof ApiError ? err.message : "Failed to delete key";
        toast.error(message);
      },
    });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Delete Service</DialogTitle>
          <DialogDescription>
            This will deactivate the service and revoke the API key. Proxied
            requests using this key will stop working. This action cannot be
            undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={handleDelete}
            disabled={deleteKey.isPending}
          >
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function LabelEditor({
  keyId,
  currentLabel,
  readOnly = false,
}: {
  readonly keyId: string;
  readonly currentLabel: string;
  readonly readOnly?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [label, setLabel] = useState(currentLabel);
  const updateKey = useUpdateKey();

  // Non-admin org members see the label but cannot edit it.
  if (readOnly) {
    return (
      <h2 className="text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
        {currentLabel}
      </h2>
    );
  }

  function handleSave() {
    const trimmed = label.trim();
    if (!trimmed || trimmed === currentLabel) {
      setLabel(currentLabel);
      setEditing(false);
      return;
    }
    updateKey.mutate(
      { keyId, label: trimmed },
      {
        onSuccess: () => {
          toast.success("Label updated");
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update label";
          toast.error(message);
        },
      },
    );
  }

  function handleCancel() {
    setLabel(currentLabel);
    setEditing(false);
  }

  if (editing) {
    return (
      <div className="flex items-center gap-2">
        <Input
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          className="text-2xl font-normal tracking-tight md:text-4xl"
          autoFocus
          onKeyDown={(e) => {
            if (e.key === "Enter") handleSave();
            if (e.key === "Escape") handleCancel();
          }}
        />
        <Button size="icon" variant="ghost" onClick={handleCancel}>
          <X className="h-4 w-4" />
        </Button>
        <Button
          size="icon"
          variant="ghost"
          onClick={handleSave}
          disabled={updateKey.isPending}
        >
          <Check className="h-4 w-4" />
        </Button>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <h2 className="text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
        {currentLabel}
      </h2>
      <Button
        size="icon"
        variant="ghost"
        onClick={() => setEditing(true)}
        className="h-8 w-8"
      >
        <Pencil className="h-4 w-4" />
      </Button>
    </div>
  );
}

function DefaultHeadersSection({
  serviceId,
  userHeaders,
  catalogHeaders,
  readOnly = false,
}: {
  readonly serviceId: string;
  readonly userHeaders: readonly DefaultRequestHeader[];
  readonly catalogHeaders: readonly DefaultRequestHeader[] | null;
  readonly readOnly?: boolean;
}) {
  // Draft only exists while editing — we seed it on Edit click and discard
  // on Save/Cancel. This keeps render pure: outside edit mode we render
  // `userHeaders` directly, which always reflects server truth.
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<readonly DefaultRequestHeader[]>([]);
  const [saveError, setSaveError] = useState<string | null>(null);
  const updateService = useUpdateUserService();

  function handleEdit() {
    setDraft(userHeaders.map((h) => ({ ...h })));
    setSaveError(null);
    setEditing(true);
  }

  function handleCancel() {
    setDraft(userHeaders);
    setSaveError(null);
    setEditing(false);
  }

  function handleSave() {
    const parsed = defaultRequestHeaderListSchema.safeParse(draft);
    if (!parsed.success) {
      const first = parsed.error.issues[0];
      setSaveError(first?.message ?? "Invalid headers");
      return;
    }
    const originalEmpty = userHeaders.length === 0;
    const nextEmpty = parsed.data.length === 0;
    // NyxID#356 tri-state: explicit clear when going from non-empty to
    // empty; otherwise replace with the list. Never send `undefined`
    // here because the user explicitly clicked Save.
    const payload = nextEmpty && !originalEmpty ? null : parsed.data;
    updateService.mutate(
      {
        serviceId,
        default_request_headers: payload,
      },
      {
        onSuccess: () => {
          toast.success("Default headers updated");
          setEditing(false);
          setSaveError(null);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update headers";
          setSaveError(message);
          toast.error(message);
        },
      },
    );
  }

  return (
    <Card className="md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <FileJson className="h-4 w-4 text-primary" />
          <CardTitle className="text-[15px]">Default request headers</CardTitle>
        </div>
        <CardDescription>
          Headers NyxID injects on every proxied request for this service.
          Non-overridable entries replace caller-supplied values; overridable
          ones yield to them. Values stored in plaintext in v1 — do not place
          real secrets here (use the key&apos;s auth method instead).
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {catalogHeaders && catalogHeaders.length > 0 && (
          <div className="space-y-2">
            <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
              From catalog (admin-configured)
            </p>
            <DefaultHeadersEditor
              value={catalogHeaders}
              onChange={() => {
                /* read-only */
              }}
              readOnly
              fromCatalog
            />
          </div>
        )}

        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
              Your headers
            </p>
            {!readOnly && !editing && (
              <Button size="icon" variant="ghost" className="h-6 w-6 shrink-0" onClick={handleEdit}>
                <Pencil className="h-3 w-3" />
              </Button>
            )}
          </div>
          {editing ? (
            <div className="space-y-2">
              <DefaultHeadersEditor
                value={draft}
                onChange={setDraft}
                disabled={updateService.isPending}
              />
              {saveError && (
                <p className="text-xs text-destructive">{saveError}</p>
              )}
              <div className="flex justify-end items-center gap-2">
                <Button
                  variant="outline"
                  onClick={handleCancel}
                  disabled={updateService.isPending}
                >
                  Cancel
                </Button>
                <Button
                  variant="primary"
                  onClick={handleSave}
                  disabled={updateService.isPending}
                >
                  Save
                </Button>
              </div>
            </div>
          ) : (
            <DefaultHeadersEditor
              value={userHeaders}
              onChange={() => {
                /* read-only when not editing */
              }}
              readOnly
            />
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function WsFrameInjectionsSection({
  serviceId,
  rules,
}: {
  readonly serviceId: string;
  readonly rules: readonly WsFrameInjection[];
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<WsFrameInjection[]>([]);
  const [saveError, setSaveError] = useState<string | null>(null);
  const updateService = useUpdateUserService();

  function handleEdit() {
    setDraft(rules.map((rule) => ({ ...rule })));
    setSaveError(null);
    setEditing(true);
  }

  function handleCancel() {
    setDraft([]);
    setSaveError(null);
    setEditing(false);
  }

  function handleSave() {
    const parsed = wsFrameInjectionsSchema.safeParse(draft);
    if (!parsed.success) {
      const first = parsed.error.issues[0];
      setSaveError(first?.message ?? "Invalid WebSocket auth-frame rules");
      return;
    }

    updateService.mutate(
      {
        serviceId,
        ws_frame_injections: parsed.data,
      },
      {
        onSuccess: () => {
          toast.success("WebSocket auth frames updated");
          setEditing(false);
          setSaveError(null);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to update WebSocket auth frames";
          setSaveError(message);
          toast.error(message);
        },
      },
    );
  }

  return (
    <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
      <div className="flex items-center justify-between border-b border-border/50 px-5 py-3">
        <div>
          <h3 className="text-[13px] font-semibold text-foreground">WebSocket auth frames</h3>
          <p className="text-[11px] text-muted-foreground mt-0.5">
            User-owned frame injection rules for post-upgrade auth.
          </p>
        </div>
        {!editing && (
          <Button size="icon" variant="ghost" className="h-6 w-6 shrink-0" onClick={handleEdit}>
            <Pencil className="h-3 w-3" />
          </Button>
        )}
      </div>

      {editing ? (
        <div className="space-y-3 p-4">
          <WsFrameInjectionsEditor
            value={draft}
            onChange={setDraft}
            errorMessage={saveError ?? undefined}
          />
          <div className="flex justify-end items-center gap-2">
            <Button
              variant="outline"
              onClick={handleCancel}
              disabled={updateService.isPending}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={handleSave}
              disabled={updateService.isPending}
            >
              Save
            </Button>
          </div>
        </div>
      ) : (
        <div className="px-5 py-4">
          <Badge variant="secondary">{rules.length}/4 rules</Badge>
          {rules.length === 0 && (
            <p className="mt-2 text-[12px] text-muted-foreground">
              No user-owned WebSocket auth-frame rules.
            </p>
          )}
        </div>
      )}
    </div>
  );
}

export function KeyDetailPage() {
  const { keyId } = useParams({ strict: false }) as { keyId: string };
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as {
    readonly provider_status?: string;
    readonly message?: string;
  };
  const { data: keyInfo, isLoading, error, refetch } = useKey(keyId);
  // Issue #416: resolve the bound node's name so the auto-connected
  // detail branch can show real routing instead of a hardcoded
  // "Direct" label. Auto-connected services don't expose a routing
  // editor (they're platform-managed), so this stays read-only here.
  const { data: nodes } = useNodes();
  const nodeName = keyInfo?.node_id
    ? (nodes?.find((n) => n.id === keyInfo.node_id)?.name ??
      keyInfo.node_id.slice(0, 8))
    : null;
  // Fetch the catalog entry by slug directly instead of scanning the
  // filtered `/catalog` listing. The list endpoint hides no-auth /
  // internal services that don't need credential setup, but a key can
  // still be backed by one of those rows (auto-provisioned) — scanning
  // the list would silently drop the inherited-defaults panel for them
  // even though the proxy still injects those defaults at request time.
  // NyxID#356 Codex review P2.
  const { data: catalogEntry } = useCatalogEntry(
    keyInfo?.catalog_service_slug ?? null,
  );
  useBreadcrumbLabel(keyInfo?.label ?? keyInfo?.catalog_service_slug);

  const [deleteOpen, setDeleteOpen] = useState(false);
  const [reconnectOpen, setReconnectOpen] = useState(false);

  const catalogHeaders = useMemo<
    readonly DefaultRequestHeader[] | null
  >(() => {
    if (!catalogEntry?.default_request_headers) return null;
    return [...catalogEntry.default_request_headers];
  }, [catalogEntry]);

  useEffect(() => {
    if (search.provider_status === "success") {
      toast.success("Service connected successfully");
      void navigate({ to: ".", search: {}, replace: true });
    } else if (search.provider_status === "error") {
      toast.error(search.message ?? "Failed to connect service");
      void navigate({ to: ".", search: {}, replace: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.provider_status]);

  if (isLoading) {
    return (
      <div className="space-y-8">
        <Skeleton className="h-20 w-full" />
        <div className="grid gap-4 md:grid-cols-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-48" />
          ))}
        </div>
      </div>
    );
  }

  if (error || !keyInfo) {
    return (
      <div className="space-y-8">
        <PageHeader
          title="Key Not Found"
        />
        <ErrorBanner
          message={
            error instanceof ApiError
              ? error.message
              : "Failed to load key details."
          }
          onRetry={refetch}
        />
      </div>
    );
  }

  const isSsh = keyInfo.service_type === "ssh";
  const hasCertAuth = isSsh && keyInfo.ssh_ca_public_key !== null;
  const sshServiceId = keyInfo.catalog_service_id;

  // Mutation gating: personal credentials and org-admin access allow edits.
  // Members, viewers, and scope-blocked rows are read-only. The backend
  // ownership helpers reject non-admin writes with 403 / NotFound, so every
  // edit control needs to match those rules -- otherwise the user gets
  // a confusing toast error after every attempt.
  const source = keyInfo.credential_source;
  const isOrgSource = source?.type === "org";
  const readOnly = isOrgSource && source.role !== "admin";
  const canReconnect =
    !readOnly &&
    !keyInfo.auto_connected &&
    RECONNECTABLE_STATUSES.has(keyInfo.status) &&
    (keyInfo.credential_type === "oauth2" ||
      catalogEntry?.provider_type === "oauth2" ||
      catalogEntry?.provider_type === "device_code");

  const sshConfig: SshServiceConfig | null =
    isSsh && keyInfo.ssh_host && keyInfo.ssh_port !== null
      ? {
          host: keyInfo.ssh_host,
          port: keyInfo.ssh_port,
          certificate_auth_enabled: hasCertAuth,
          certificate_ttl_minutes: keyInfo.ssh_certificate_ttl_minutes ?? 30,
          allowed_principals: keyInfo.ssh_allowed_principals
            ? [...keyInfo.ssh_allowed_principals]
            : [],
          ca_public_key: keyInfo.ssh_ca_public_key,
        }
      : null;

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-2">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex flex-col gap-2">
            {keyInfo.auto_connected ? (
              <h2 className="text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
                {keyInfo.label}
              </h2>
            ) : (
              <LabelEditor
                keyId={keyInfo.id}
                currentLabel={keyInfo.label}
                readOnly={readOnly}
              />
            )}
            <div className="flex items-center gap-2">
              <p className="text-[12px] text-muted-foreground">
                {keyInfo.catalog_service_name
                  ? `${keyInfo.catalog_service_name} -- /proxy/s/${keyInfo.slug}`
                  : `/proxy/s/${keyInfo.slug}`}
              </p>
              {keyInfo.auto_connected && (
                <Badge variant="secondary">
                  {keyInfo.source_app_name
                    ? `Connected via ${keyInfo.source_app_name}`
                    : "Auto-connected"}
                </Badge>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2">
            {hasCertAuth && sshServiceId && (
              <Button
                variant="outline"
                className="text-text-tertiary hover:text-muted-foreground"
                onClick={() =>
                  void navigate({
                    to: "/ssh/$serviceId/terminal",
                    params: { serviceId: sshServiceId },
                    search: {
                      principal: keyInfo.ssh_allowed_principals?.[0],
                      returnKeyId: keyInfo.id,
                    },
                  })
                }
              >
                <ButtonIcon><Terminal className="h-4 w-4" /></ButtonIcon>
                Terminal
              </Button>
            )}
            {canReconnect && (
              <Button
                variant="primary"
                onClick={() => setReconnectOpen(true)}
              >
                <ButtonIcon variant="primary"><RefreshCw className="h-4 w-4" /></ButtonIcon>
                {reconnectLabel(keyInfo.status)}
              </Button>
            )}
            {!keyInfo.auto_connected && !readOnly && (
              <Button
                variant="destructive"
                onClick={() => setDeleteOpen(true)}
              >
                <ButtonIcon variant="destructive"><Trash2 className="h-4 w-4 text-destructive" /></ButtonIcon>
                Delete
              </Button>
            )}
          </div>
        </div>
        {readOnly && source?.type === "org" && (
          <div className="flex items-center gap-3 rounded-xl border border-success/15 bg-success/[0.04] px-4 py-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-success/10">
              <Shield className="h-4.5 w-4.5 text-success" />
            </div>
            <div>
              <p className="text-[13px] font-semibold text-foreground">
                Shared from {source.org_name}
              </p>
              <p className="text-[11px] text-muted-foreground">
                You are a {source.role} of this organization and can
                {source.allowed
                  ? " use this credential through the proxy, but only admins can modify it."
                  : " see this service but not use it. Ask an admin to grant you member access."}
              </p>
            </div>
          </div>
        )}
      </div>

      {keyInfo.auto_connected ? (
        <>
          <Card>
            <CardHeader>
              <CardTitle className="text-[15px]">Service Details</CardTitle>
              <CardDescription>
                {keyInfo.source_app_name
                  ? `This service was auto-connected via ${keyInfo.source_app_name}. It is managed by the platform and cannot be modified.`
                  : "This service requires no authentication and was auto-connected from the catalog. It is managed by the platform and cannot be modified."}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="grid grid-cols-2 gap-4 text-[12px]">
                <div>
                  <span className="text-xs font-medium text-muted-foreground">
                    Endpoint
                  </span>
                  <p className="truncate text-xs">
                    {keyInfo.endpoint_url}
                  </p>
                </div>
                <div>
                  <span className="text-xs font-medium text-muted-foreground">
                    Proxy Path
                  </span>
                  <p className="text-xs">/proxy/s/{keyInfo.slug}</p>
                </div>
                <div>
                  <span className="text-xs font-medium text-muted-foreground">
                    Auth Method
                  </span>
                  <p className="text-xs">None (no credentials required)</p>
                </div>
                <div>
                  <span className="text-xs font-medium text-muted-foreground">
                    Routing
                  </span>
                  {/*
                    Issue #416: read the actual routing rather than
                    hardcoding "Direct". Auto-connected keys are
                    platform-managed so the editor stays absent here,
                    but the display now reflects reality if the key
                    happens to be node-routed.
                  */}
                  <p className="text-xs">
                    {nodeName ? `Via ${nodeName}` : "Direct"}
                  </p>
                </div>
              </div>
            </CardContent>
          </Card>

          {/* Auto-connected keys still have catalog-level default headers
              applied at proxy time (NyxID#356). Surface them read-only so
              users can see why those headers reach the downstream — without
              this, the panel only renders for user-managed keys and the
              auto-connected case appears to have no defaults. */}
          {!isSsh && catalogHeaders && catalogHeaders.length > 0 && (
            <DefaultHeadersSection
              serviceId={keyInfo.id}
              userHeaders={[]}
              catalogHeaders={catalogHeaders}
              readOnly
            />
          )}
        </>
      ) : (
        <div className="space-y-4">
          {/* Row 1: Endpoint + OpenAPI Spec (or just Endpoint for SSH) */}
          {isSsh ? (
            <EndpointSection
              endpointUrl={keyInfo.endpoint_url}
              endpointId={keyInfo.endpoint_id}
              nodeRouted={keyInfo.node_id !== null}
              readOnly={readOnly}
            />
          ) : (
            <div className="grid gap-4 md:grid-cols-2">
              <EndpointSection
                endpointUrl={keyInfo.endpoint_url}
                endpointId={keyInfo.endpoint_id}
                nodeRouted={keyInfo.node_id !== null}
                readOnly={readOnly}
              />
              <OpenApiSpecSection
                endpointId={keyInfo.endpoint_id}
                specUrl={keyInfo.openapi_spec_url ?? null}
                readOnly={readOnly}
              />
            </div>
          )}

          {/* Row 2: Credential + Service (or SSH Connection) */}
          {keyInfo.service_type === "ssh" &&
          keyInfo.ssh_host &&
          keyInfo.ssh_port !== null ? (
            <SshConnectionSection
              sshHost={keyInfo.ssh_host}
              sshPort={keyInfo.ssh_port}
              caPublicKey={keyInfo.ssh_ca_public_key}
              principals={keyInfo.ssh_allowed_principals}
              certTtlMinutes={keyInfo.ssh_certificate_ttl_minutes}
            />
          ) : (
            <div className="grid gap-4 md:grid-cols-2">
              {keyInfo.api_key_id && (
                <ApiKeySection
                  apiKeyId={keyInfo.api_key_id}
                  credentialType={keyInfo.credential_type}
                  status={keyInfo.status}
                  expiresAt={keyInfo.expires_at}
                  lastUsedAt={keyInfo.last_used_at}
                  errorMessage={keyInfo.error_message}
                  readOnly={readOnly}
                />
              )}
              <ServiceSection
                slug={keyInfo.slug}
                authMethod={keyInfo.auth_method}
                authKeyName={keyInfo.auth_key_name}
                isActive={keyInfo.is_active}
                credentialStatus={keyInfo.status}
                hasCredential={keyInfo.api_key_id !== null && keyInfo.api_key_id !== undefined}
                serviceId={keyInfo.id}
                customUserAgent={keyInfo.custom_user_agent}
                nodeId={keyInfo.node_id}
                nodeStatus={keyInfo.node_status}
                readOnly={readOnly}
              />
            </div>
          )}

          {/* Routing */}
          <RoutingSection
            nodeId={keyInfo.node_id}
            serviceId={keyInfo.id}
            readOnly={readOnly}
          />

          {/* Default Headers (non-SSH) */}
          {!isSsh && (
            <DefaultHeadersSection
              serviceId={keyInfo.id}
              userHeaders={
                keyInfo.default_request_headers
                  ? [...keyInfo.default_request_headers]
                  : []
              }
              catalogHeaders={catalogHeaders}
              readOnly={readOnly}
            />
          )}

          {/* WS Frame Injections (non-SSH, non-readOnly) */}
          {!isSsh && !readOnly && (
            <WsFrameInjectionsSection
              serviceId={keyInfo.id}
              rules={
                keyInfo.ws_frame_injections
                  ? [...keyInfo.ws_frame_injections]
                  : []
              }
            />
          )}

          {/* Lark Permission Setup */}
          {keyInfo.permission_setup_url && (
            <LarkPermissionSetupCard
              url={keyInfo.permission_setup_url}
              scopes={keyInfo.permission_setup_scopes ?? []}
            />
          )}

          {/* Node Setup Helper */}
          {keyInfo.node_id && !isSsh && (
            <NodeSetupHelper
              slug={keyInfo.slug}
              endpointUrl={keyInfo.endpoint_url}
              authMethod={keyInfo.auth_method}
              authKeyName={keyInfo.auth_key_name}
              catalogServiceName={keyInfo.catalog_service_name}
            />
          )}

          {/* API Usage */}
          {!isSsh && (
            <ApiUsageSection
              slug={keyInfo.slug}
              authMethod={keyInfo.auth_method}
              endpointUrl={keyInfo.endpoint_url}
              catalogServiceSlug={keyInfo.catalog_service_slug}
              label={keyInfo.label}
              catalogEntry={catalogEntry}
            />
          )}
        </div>
      )}

      {keyInfo.auto_connected && (
        <ApiUsageSection
          slug={keyInfo.slug}
          authMethod={keyInfo.auth_method}
          endpointUrl={keyInfo.endpoint_url}
          catalogServiceSlug={keyInfo.catalog_service_slug}
          label={keyInfo.label}
          catalogEntry={catalogEntry}
        />
      )}

      {sshConfig && sshServiceId && (
        <Card>
          <CardHeader>
            <CardTitle>Connection Instructions</CardTitle>
            <CardDescription>
              How to connect to this SSH service
            </CardDescription>
          </CardHeader>
          <CardContent>
            <SshServiceInstructions
              serviceId={sshServiceId}
              serviceSlug={keyInfo.slug}
              sshConfig={sshConfig}
            />
          </CardContent>
        </Card>
      )}

      <DeleteKeyDialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        keyId={keyInfo.id}
      />
      <AddKeyDialog
        open={reconnectOpen}
        onOpenChange={setReconnectOpen}
        reconnectKey={keyInfo}
      />
    </div>
  );
}
