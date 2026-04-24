import { useEffect, useMemo, useState } from "react";
import { useParams, useNavigate, useSearch } from "@tanstack/react-router";
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
import { deriveServiceBadge } from "@/lib/service-status";
import { copyToClipboard } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { Breadcrumb } from "@/components/shared/breadcrumb";
import { SshServiceInstructions } from "@/components/dashboard/ssh-service-instructions";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
  Router,
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
} from "lucide-react";
import { toast } from "sonner";
import type { SshServiceConfig } from "@/types/api";

function statusVariant(
  status: string,
): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "active":
      return "default";
    case "expired":
      return "secondary";
    case "revoked":
    case "refresh_failed":
      return "destructive";
    default:
      return "outline";
  }
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
        <div className="flex items-center gap-2">
          <ShieldCheck className="h-4 w-4 text-text-tertiary" />
          <CardTitle className="text-sm">Configure Permissions</CardTitle>
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
                  <Badge variant="outline" className="font-mono text-xs">
                    {scope}
                  </Badge>
                </li>
              ))}
            </ul>
          </div>
        )}
        <Button asChild size="sm">
          <a href={url} target="_blank" rel="noopener noreferrer">
            Open Permissions Page
            <ExternalLink className="ml-2 h-4 w-4" />
          </a>
        </Button>
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
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Globe className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">Endpoint</CardTitle>
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
              className="flex-1 font-mono text-sm"
            />
            <Button
              size="icon"
              variant="ghost"
              onClick={handleSave}
              disabled={updateEndpoint.isPending}
            >
              <Check className="h-4 w-4" />
            </Button>
            <Button size="icon" variant="ghost" onClick={handleCancel}>
              <X className="h-4 w-4" />
            </Button>
          </div>
        ) : isEmpty && nodeRouted ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <span>Resolved by node agent</span>
            {!readOnly && (
              <Button
                size="icon"
                variant="ghost"
                onClick={() => setEditing(true)}
              >
                <Pencil className="h-4 w-4" />
              </Button>
            )}
          </div>
        ) : (
          <div className="flex items-center justify-between gap-2">
            <code className="truncate rounded bg-muted px-2 py-1 font-mono text-sm">
              {endpointUrl}
            </code>
            {!readOnly && (
              <Button
                size="icon"
                variant="ghost"
                onClick={() => setEditing(true)}
              >
                <Pencil className="h-4 w-4" />
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
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <FileJson className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">OpenAPI Spec</CardTitle>
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
              className="flex-1 font-mono text-sm"
              type="url"
            />
            <Button
              size="icon"
              variant="ghost"
              onClick={handleSave}
              disabled={updateEndpoint.isPending}
            >
              <Check className="h-4 w-4" />
            </Button>
            <Button size="icon" variant="ghost" onClick={handleCancel}>
              <X className="h-4 w-4" />
            </Button>
          </div>
        ) : specUrl ? (
          <div className="flex items-center justify-between gap-2">
            <code className="truncate rounded bg-muted px-2 py-1 font-mono text-sm">
              {specUrl}
            </code>
            {!readOnly && (
              <Button size="icon" variant="ghost" onClick={handleEdit}>
                <Pencil className="h-4 w-4" />
              </Button>
            )}
          </div>
        ) : (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <span>Not set</span>
            {!readOnly && (
              <Button size="icon" variant="ghost" onClick={handleEdit}>
                <Pencil className="h-4 w-4" />
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

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <KeyRound className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">API Key</CardTitle>
        </div>
        <CardDescription>Authentication credential</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-3">
          <Badge variant={statusVariant(status)}>{status}</Badge>
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
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={handleRotate}
                disabled={updateApiKey.isPending || !newCredential.trim()}
              >
                Save
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setRotating(false);
                  setNewCredential("");
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <Button size="sm" variant="outline" onClick={() => setRotating(true)}>
            <RefreshCw className="mr-2 h-3 w-3" />
            Rotate Credential
          </Button>
        )}
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
  readOnly = false,
}: {
  readonly slug: string;
  readonly authMethod: string;
  readonly authKeyName: string;
  readonly isActive: boolean;
  /** API key status ("active" | "pending_auth" | "expired" | "revoked" |
   *  "refresh_failed"). When the service has no credential (e.g. auto-connected
   *  downstreams) this is ignored. */
  readonly credentialStatus: string;
  /** Whether this service has an associated credential. Services without a
   *  credential (e.g. auto-connected or no-auth downstreams) skip the
   *  credential-readiness check entirely. */
  readonly hasCredential: boolean;
  readonly serviceId: string;
  readonly customUserAgent?: string | null;
  readonly readOnly?: boolean;
}) {
  const updateService = useUpdateUserService();
  const [editingUa, setEditingUa] = useState(false);
  const [uaDraft, setUaDraft] = useState(customUserAgent ?? "");

  const { variant: badgeVariant, label: badgeLabel, credentialBlocked } =
    deriveServiceBadge({ isActive, credentialStatus, hasCredential });

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
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Server className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">Service</CardTitle>
        </div>
        <CardDescription>Proxy routing configuration</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-3">
          <code className="rounded bg-muted px-2 py-1 font-mono text-sm">
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
            <div className="flex items-center gap-2">
              <span className="text-xs">
                {customUserAgent || (
                  <span className="text-muted-foreground">
                    Passthrough (default)
                  </span>
                )}
              </span>
              {!readOnly && (
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6"
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

        {!readOnly && (
          <Button
            size="sm"
            variant="outline"
            onClick={toggleActive}
            disabled={updateService.isPending}
          >
            {isActive ? "Deactivate" : "Activate"}
          </Button>
        )}
      </CardContent>
    </Card>
  );
}

function RoutingSection({
  nodeId,
  serviceId,
  readOnly = false,
}: {
  readonly nodeId: string | null;
  readonly serviceId: string;
  readonly readOnly?: boolean;
}) {
  const [picking, setPicking] = useState(false);
  const { data: nodes } = useNodes();
  const updateService = useUpdateUserService();

  function handleSelectNode(selectedNodeId: string) {
    const id = selectedNodeId === "direct" ? "" : selectedNodeId;
    updateService.mutate(
      { serviceId, node_id: id },
      {
        onSuccess: () => {
          toast.success(id ? "Route updated" : "Switched to direct routing");
          setPicking(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError ? err.message : "Failed to update routing";
          toast.error(message);
        },
      },
    );
  }

  const allNodes = nodes ?? [];
  const currentNodeName = nodeId
    ? (nodes?.find((n) => n.id === nodeId)?.name ?? nodeId)
    : null;

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Router className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">Routing</CardTitle>
        </div>
        <CardDescription>How requests reach the endpoint</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-2">
          <Badge variant={nodeId ? "default" : "outline"}>
            {nodeId ? `Via node: ${currentNodeName}` : "Direct"}
          </Badge>
        </div>

        {!readOnly && picking ? (
          <div className="space-y-2">
            <Label className="text-xs">Select routing</Label>
            <Select
              onValueChange={handleSelectNode}
              defaultValue={nodeId ?? "direct"}
            >
              <SelectTrigger>
                <SelectValue placeholder="Select routing" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="direct">Direct (no node)</SelectItem>
                {allNodes.map((n) => (
                  <SelectItem
                    key={n.id}
                    value={n.id}
                    disabled={n.status !== "online"}
                  >
                    {n.name} ({n.status})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {allNodes.length === 0 && (
              <p className="text-xs text-muted-foreground">
                No nodes registered. Register a node first.
              </p>
            )}
            <Button
              size="sm"
              variant="outline"
              onClick={() => setPicking(false)}
            >
              Cancel
            </Button>
          </div>
        ) : !readOnly ? (
          <Button size="sm" variant="outline" onClick={() => setPicking(true)}>
            {nodeId ? "Change Route" : "Route via Node"}
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}

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
    <Card className="md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Terminal className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">Node Setup</CardTitle>
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
          <pre className="overflow-x-auto rounded-lg bg-muted p-3 font-mono text-xs leading-relaxed">
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
          <pre className="overflow-x-auto rounded-lg bg-muted p-3 font-mono text-xs leading-relaxed">
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
          <CardTitle className="text-sm">SSH Connection</CardTitle>
        </div>
        <CardDescription>
          SSH certificate authentication details
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <span className="text-xs font-medium text-muted-foreground">
              Host
            </span>
            <p className="font-mono">
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
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-muted-foreground">
                CA Public Key
              </span>
              <Button
                size="sm"
                variant="ghost"
                onClick={handleCopyCa}
                className="h-6 px-2"
              >
                <Copy className="mr-1 h-3 w-3" />
                <span className="text-xs">Copy</span>
              </Button>
            </div>
            <pre className="mt-1 overflow-x-auto rounded-lg bg-muted p-3 font-mono text-xs leading-relaxed">
              {caPublicKey}
            </pre>
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

function ApiUsageSection({
  slug,
  authMethod,
}: {
  readonly slug: string;
  readonly authMethod: string;
}) {
  const proxyUrl = `${window.location.origin}/api/v1/proxy/s/${slug}`;

  const authNote =
    authMethod === "none"
      ? "This service requires no upstream credentials, but you still need to authenticate with NyxID."
      : "NyxID injects your stored credentials automatically when proxying.";

  const curlExample = [
    `curl ${proxyUrl}/v1/chat/completions \\`,
    `  -H "Authorization: Bearer <NYXID_ACCESS_TOKEN>" \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}'`,
  ].join("\n");

  const apiKeyExample = [
    `curl ${proxyUrl}/v1/chat/completions \\`,
    `  -H "X-API-Key: nyx_..." \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}'`,
  ].join("\n");

  function handleCopyUrl() {
    void copyToClipboard(proxyUrl).then(() => {
      toast.success("Proxy URL copied");
    });
  }

  function handleCopyCurl() {
    void copyToClipboard(apiKeyExample).then(() => {
      toast.success("Example copied");
    });
  }

  return (
    <Card className="md:col-span-2">
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Code className="h-4 w-4 text-primary" />
          <CardTitle className="text-sm">API Usage</CardTitle>
        </div>
        <CardDescription>
          How to connect to this service through NyxID proxy
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div>
          <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">
            Base URL
          </p>
          <div className="relative">
            <pre className="overflow-x-auto rounded-lg bg-muted p-3 pr-10 font-mono text-sm">
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
            Append the downstream API path after this URL (e.g.{" "}
            <code className="rounded bg-background px-1">
              /v1/chat/completions
            </code>
            ). {authNote}
          </p>
        </div>

        <div>
          <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">
            Authentication
          </p>
          <div className="space-y-2 text-xs text-muted-foreground">
            <p>
              Authenticate with NyxID using one of these methods:
            </p>
            <ul className="list-disc list-inside space-y-1 pl-1">
              <li>
                <span className="font-medium text-foreground">API Key:</span>{" "}
                <code className="rounded bg-background px-1">
                  X-API-Key: nyx_...
                </code>{" "}
                header (create one in API Keys tab)
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

        <div>
          <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">
            Example (with API key)
          </p>
          <div className="relative">
            <pre className="overflow-x-auto rounded-lg bg-muted p-3 pr-10 font-mono text-xs leading-relaxed">
              {apiKeyExample}
            </pre>
            <Button
              size="icon"
              variant="ghost"
              className="absolute right-2 top-2 h-7 w-7"
              onClick={handleCopyCurl}
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>

        <div>
          <p className="mb-1.5 text-[11px] font-medium text-muted-foreground">
            Example (with Bearer token)
          </p>
          <pre className="overflow-x-auto rounded-lg bg-muted p-3 font-mono text-xs leading-relaxed">
            {curlExample}
          </pre>
        </div>
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
      <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">
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
          className="font-display text-2xl font-normal tracking-tight md:text-4xl"
          autoFocus
          onKeyDown={(e) => {
            if (e.key === "Enter") handleSave();
            if (e.key === "Escape") handleCancel();
          }}
        />
        <Button
          size="icon"
          variant="ghost"
          onClick={handleSave}
          disabled={updateKey.isPending}
        >
          <Check className="h-4 w-4" />
        </Button>
        <Button size="icon" variant="ghost" onClick={handleCancel}>
          <X className="h-4 w-4" />
        </Button>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">
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
          <CardTitle className="text-sm">Default request headers</CardTitle>
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
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
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
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              Your headers
            </p>
            {!readOnly && !editing && (
              <Button size="sm" variant="outline" onClick={handleEdit}>
                <Pencil className="mr-1 h-3 w-3" />
                Edit
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
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  onClick={handleSave}
                  disabled={updateService.isPending}
                >
                  Save
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={handleCancel}
                  disabled={updateService.isPending}
                >
                  Cancel
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
    <div className="space-y-3 md:col-span-2">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold">WebSocket auth frames</h3>
          <p className="text-xs text-muted-foreground">
            User-owned frame injection rules for post-upgrade auth.
          </p>
        </div>
        {!editing && (
          <Button size="sm" variant="outline" onClick={handleEdit}>
            <Pencil className="mr-1 h-3 w-3" />
            Edit
          </Button>
        )}
      </div>

      {editing ? (
        <div className="space-y-3">
          <WsFrameInjectionsEditor
            value={draft}
            onChange={setDraft}
            errorMessage={saveError ?? undefined}
          />
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              onClick={handleSave}
              disabled={updateService.isPending}
            >
              Save
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={handleCancel}
              disabled={updateService.isPending}
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <div className="rounded-[10px] border border-border p-3">
          <Badge variant="outline">{rules.length}/4 rules</Badge>
          {rules.length === 0 && (
            <p className="mt-2 text-xs text-muted-foreground">
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
  const { data: keyInfo, isLoading, error } = useKey(keyId);
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
  const [deleteOpen, setDeleteOpen] = useState(false);

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
          breadcrumbs={[
            { label: "AI Services", to: "/keys" },
            { label: "Not Found" },
          ]}
        />
        <Card>
          <CardContent className="py-8 text-center text-sm text-destructive">
            {error instanceof ApiError
              ? error.message
              : "Failed to load key details."}
          </CardContent>
        </Card>
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
        <Breadcrumb
          items={[
            { label: "AI Services", to: "/keys" },
            { label: keyInfo.label },
          ]}
        />
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex flex-col gap-2">
            {keyInfo.auto_connected ? (
              <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">
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
              <p className="text-sm text-muted-foreground">
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
                size="sm"
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
                <Terminal className="mr-2 h-4 w-4" />
                Terminal
              </Button>
            )}
            {!keyInfo.auto_connected && !readOnly && (
              <Button
                variant="destructive"
                size="sm"
                onClick={() => setDeleteOpen(true)}
              >
                <Trash2 className="mr-2 h-4 w-4" />
                Delete
              </Button>
            )}
          </div>
        </div>
        {readOnly && source?.type === "org" && (
          <Card className="border-info/40 bg-info/5">
            <CardContent className="flex items-start gap-3 py-3">
              <Shield className="mt-0.5 h-4 w-4 shrink-0 text-info" />
              <div className="text-xs">
                <p className="font-medium text-foreground">
                  Shared from {source.org_name}
                </p>
                <p className="text-muted-foreground">
                  You are a {source.role} of this organization and can
                  {source.allowed
                    ? " use this credential through the proxy, but only admins can modify it."
                    : " see this service but not use it. Ask an admin to grant you member access."}
                </p>
              </div>
            </CardContent>
          </Card>
        )}
      </div>

      {keyInfo.auto_connected ? (
        <>
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Service Details</CardTitle>
              <CardDescription>
                {keyInfo.source_app_name
                  ? `This service was auto-connected via ${keyInfo.source_app_name}. It is managed by the platform and cannot be modified.`
                  : "This service requires no authentication and was auto-connected from the catalog. It is managed by the platform and cannot be modified."}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="grid grid-cols-2 gap-4 text-sm">
                <div>
                  <span className="text-xs font-medium text-muted-foreground">
                    Endpoint
                  </span>
                  <p className="truncate font-mono text-xs">
                    {keyInfo.endpoint_url}
                  </p>
                </div>
                <div>
                  <span className="text-xs font-medium text-muted-foreground">
                    Proxy Path
                  </span>
                  <p className="font-mono text-xs">/proxy/s/{keyInfo.slug}</p>
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
                  <p className="text-xs">Direct</p>
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
        <div className="grid gap-4 md:grid-cols-2">
          <EndpointSection
            endpointUrl={keyInfo.endpoint_url}
            endpointId={keyInfo.endpoint_id}
            nodeRouted={keyInfo.node_id !== null}
            readOnly={readOnly}
          />

          {keyInfo.service_type !== "ssh" && (
            <OpenApiSpecSection
              endpointId={keyInfo.endpoint_id}
              specUrl={keyInfo.openapi_spec_url ?? null}
              readOnly={readOnly}
            />
          )}

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
          ) : keyInfo.api_key_id ? (
            <ApiKeySection
              apiKeyId={keyInfo.api_key_id}
              credentialType={keyInfo.credential_type}
              status={keyInfo.status}
              expiresAt={keyInfo.expires_at}
              lastUsedAt={keyInfo.last_used_at}
              errorMessage={keyInfo.error_message}
              readOnly={readOnly}
            />
          ) : null}

          <ServiceSection
            slug={keyInfo.slug}
            authMethod={keyInfo.auth_method}
            authKeyName={keyInfo.auth_key_name}
            isActive={keyInfo.is_active}
            credentialStatus={keyInfo.status}
            hasCredential={keyInfo.api_key_id !== null && keyInfo.api_key_id !== undefined}
            serviceId={keyInfo.id}
            customUserAgent={keyInfo.custom_user_agent}
            readOnly={readOnly}
          />

          <RoutingSection
            nodeId={keyInfo.node_id}
            serviceId={keyInfo.id}
            readOnly={readOnly}
          />

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

          {keyInfo.permission_setup_url && (
            <LarkPermissionSetupCard
              url={keyInfo.permission_setup_url}
              scopes={keyInfo.permission_setup_scopes ?? []}
            />
          )}

          {keyInfo.node_id && !isSsh && (
          <NodeSetupHelper
            slug={keyInfo.slug}
            endpointUrl={keyInfo.endpoint_url}
            authMethod={keyInfo.auth_method}
            authKeyName={keyInfo.auth_key_name}
            catalogServiceName={keyInfo.catalog_service_name}
          />
          )}

          {!isSsh && (
            <ApiUsageSection
              slug={keyInfo.slug}
              authMethod={keyInfo.auth_method}
            />
          )}
        </div>
      )}

      {keyInfo.auto_connected && (
        <ApiUsageSection
          slug={keyInfo.slug}
          authMethod={keyInfo.auth_method}
        />
      )}

      {sshConfig && sshServiceId && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Connection Instructions</CardTitle>
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
    </div>
  );
}
