import { useState, useEffect, useRef, useCallback } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useCatalog, useCreateKey } from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import {
  useInitiateOAuth,
  useInitiateDeviceCode,
  usePollDeviceCode,
  useSetProviderCredentials,
} from "@/hooks/use-providers";
import { ApiError } from "@/lib/api-client";
import { hardRedirect } from "@/lib/navigation";
import { copyToClipboard } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  ArrowLeft,
  ExternalLink,
  Globe,
  Search,
  Loader2,
  Copy,
  CheckCircle2,
  AlertCircle,
  Server,
  Terminal,
} from "lucide-react";
import { toast } from "sonner";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
import type { DeviceCodePollResponse } from "@/types/api";

type WizardStep =
  | "catalog"
  | "routing"
  | "form"
  | "node_setup"
  | "oauth_credentials"
  | "oauth"
  | "device_code";

interface FormState {
  readonly credential: string;
  readonly label: string;
  readonly endpointUrl: string;
  readonly slug: string;
  readonly authMethod: string;
  readonly authKeyName: string;
  readonly nodeId: string;
  readonly serviceType: string;
  readonly sshHost: string;
  readonly sshPort: string;
  readonly sshCertificateAuth: boolean;
  readonly sshPrincipals: string;
  readonly sshCertificateTtlMinutes: string;
  /** Optional OpenAPI spec URL — enables endpoint discovery for AI tools. */
  readonly openapiSpecUrl: string;
}

const AUTH_METHOD_DEFAULTS: Record<string, string> = {
  bearer: "Authorization",
  header: "X-API-Key",
  query: "key",
  path: "bot",
  basic: "Authorization",
  oidc: "Authorization",
  oauth2: "Authorization",
  body: "app_secret",
  bot_bearer: "Authorization",
  token_exchange: "",
  none: "",
};

// Derive a user-friendly credential field label/placeholder from the auth
// method + key name. Prevents confusing "API Key / Credential" + "sk-..." for
// body-injected credentials (e.g. Lark's `app_secret`) or Discord bot tokens.
function getCredentialFieldMeta(
  authMethod: string,
  authKeyName: string,
): { readonly label: string; readonly placeholder: string } {
  if (authMethod === "bot_bearer") {
    return { label: "Bot Token", placeholder: "Discord bot token" };
  }
  if (authMethod === "body") {
    const fieldName = authKeyName.trim();
    if (fieldName) {
      const pretty = fieldName
        .split(/[_\-\s]+/)
        .filter(Boolean)
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join(" ");
      return { label: pretty || "Credential", placeholder: `Enter ${fieldName}` };
    }
    return { label: "Credential", placeholder: "Enter credential value" };
  }
  if (authMethod === "basic") {
    return { label: "Username:Password", placeholder: "user:pass" };
  }
  return { label: "API Key / Credential", placeholder: "sk-..." };
}

// Parse a stored `token_exchange` credential (JSON object) back into a
// plain key-value map. Returns an empty object on any parse error so the
// form stays usable even if the underlying JSON is mangled.
function parseTokenExchangeCredential(
  credential: string,
): Readonly<Record<string, string>> {
  if (!credential) return {};
  try {
    const parsed = JSON.parse(credential);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const result: Record<string, string> = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof v === "string") result[k] = v;
      }
      return result;
    }
  } catch {
    // Fall through -- treat malformed JSON as empty.
  }
  return {};
}

// Compose a map of field values back into the JSON blob the backend
// expects in `credential_encrypted`. Returns an empty string when all
// fields are blank so the surrounding "required credential" check still
// fires.
function composeTokenExchangeCredential(
  values: Readonly<Record<string, string>>,
): string {
  const trimmed: Record<string, string> = {};
  let anyPresent = false;
  for (const [k, v] of Object.entries(values)) {
    const t = v.trim();
    if (t) anyPresent = true;
    trimmed[k] = t;
  }
  if (!anyPresent) return "";
  return JSON.stringify(trimmed);
}

// Auth key name input should only be shown when the user needs to pick a
// header/query/body field name. `bot_bearer` is a fixed Authorization format,
// OAuth flows handle their own token storage, and `token_exchange` ignores
// auth_key_name entirely (the credential is a structured JSON blob).
function shouldShowAuthKeyName(authMethod: string): boolean {
  return (
    authMethod !== "none" &&
    authMethod !== "oidc" &&
    authMethod !== "oauth2" &&
    authMethod !== "bot_bearer" &&
    authMethod !== "token_exchange"
  );
}

function getAuthKeyNameLabel(authMethod: string): string {
  if (authMethod === "body") return "Body Field Name";
  if (authMethod === "header") return "Header Name";
  if (authMethod === "query") return "Query Parameter";
  return "Auth Key Name";
}

const INITIAL_FORM: FormState = {
  credential: "",
  label: "",
  endpointUrl: "",
  slug: "",
  authMethod: "bearer",
  authKeyName: "Authorization",
  nodeId: "",
  serviceType: "http",
  sshHost: "",
  sshPort: "22",
  sshCertificateAuth: true,
  sshPrincipals: "",
  sshCertificateTtlMinutes: "30",
  openapiSpecUrl: "",
};

function CopyableCode({ children }: { readonly children: string }) {
  function handleCopy() {
    void copyToClipboard(children).then(() => {
      toast.success("Copied to clipboard");
    });
  }

  return (
    <div className="relative">
      <pre className="overflow-x-auto rounded-lg bg-muted p-3 pr-10 font-mono text-xs leading-relaxed">
        {children}
      </pre>
      <Button
        size="icon"
        variant="ghost"
        className="absolute right-2 top-2 h-7 w-7"
        onClick={handleCopy}
      >
        <Copy className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

function CatalogGrid({
  onSelect,
  onCustom,
  onCustomSsh,
}: {
  readonly onSelect: (entry: CatalogEntry) => void;
  readonly onCustom: () => void;
  readonly onCustomSsh: () => void;
}) {
  const { data: entries, isLoading } = useCatalog();
  const [search, setSearch] = useState("");

  const filtered = entries?.filter(
    (e) =>
      e.name.toLowerCase().includes(search.toLowerCase()) ||
      e.slug.toLowerCase().includes(search.toLowerCase()),
  );

  if (isLoading) {
    return (
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {Array.from({ length: 9 }, (_, i) => (
          <Skeleton key={i} className="h-20 rounded-lg" />
        ))}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="relative">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          placeholder="Search services..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="pl-9"
        />
      </div>

      <div className="grid max-h-[380px] grid-cols-2 gap-3 overflow-y-auto pr-1 sm:grid-cols-3">
        <button
          type="button"
          onClick={onCustom}
          className="flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-border p-4 text-center transition-colors hover:border-primary/40 hover:bg-accent/40"
        >
          <Globe className="h-5 w-5 text-muted-foreground" />
          <span className="text-xs font-medium">Custom Endpoint</span>
        </button>

        <button
          type="button"
          onClick={onCustomSsh}
          className="flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-border p-4 text-center transition-colors hover:border-primary/40 hover:bg-accent/40"
        >
          <Terminal className="h-5 w-5 text-muted-foreground" />
          <span className="text-xs font-medium">Custom SSH</span>
        </button>

        {filtered?.map((entry) => (
          <button
            key={entry.slug}
            type="button"
            onClick={() => onSelect(entry)}
            className="flex flex-col items-start gap-1.5 rounded-lg border border-border p-4 text-left transition-colors hover:border-primary/40 hover:bg-accent/40"
          >
            <span className="text-sm font-medium">{entry.name}</span>
            <span className="line-clamp-2 text-[11px] text-muted-foreground">
              {entry.description ?? entry.base_url}
            </span>
            {entry.service_type === "ssh" && (
              <Badge variant="secondary" className="text-[10px]">
                SSH
              </Badge>
            )}
            {entry.requires_gateway_url && (
              <Badge variant="outline" className="text-[10px]">
                URL required
              </Badge>
            )}
            {entry.provider_type === "oauth2" && (
              <Badge variant="secondary" className="text-[10px]">
                OAuth
              </Badge>
            )}
            {entry.provider_type === "device_code" && (
              <Badge variant="secondary" className="text-[10px]">
                Device Code
              </Badge>
            )}
          </button>
        ))}
      </div>
    </div>
  );
}

function RoutingStep({
  catalogEntry,
  form,
  onChange,
  onDirect,
  onViaNode,
  onBack,
  isSshOnly,
}: {
  readonly catalogEntry: CatalogEntry | null;
  readonly form: FormState;
  readonly onChange: (updates: Partial<FormState>) => void;
  readonly onDirect: () => void;
  readonly onViaNode: () => void;
  readonly onBack: () => void;
  readonly isSshOnly: boolean;
}) {
  const { data: nodes, isLoading } = useNodes();
  const onlineNodes = nodes?.filter((n) => n.status === "online") ?? [];
  const [routingChoice, setRoutingChoice] = useState<"direct" | "node">(
    isSshOnly ? "node" : "direct",
  );

  function handleNext() {
    if (routingChoice === "node" && !form.nodeId) return;
    if (routingChoice === "node") {
      onViaNode();
    } else {
      onDirect();
    }
  }

  return (
    <div className="space-y-4">
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3 w-3" />
        Back to catalog
      </button>

      {catalogEntry && (
        <div className="rounded-lg border border-border bg-muted/50 p-3">
          <p className="text-sm font-medium">{catalogEntry.name}</p>
          {catalogEntry.description && (
            <p className="text-xs text-muted-foreground">
              {catalogEntry.description}
            </p>
          )}
        </div>
      )}

      <div className="space-y-3">
        <Label>How should requests reach this service?</Label>

        {!isSshOnly ? (
          <div className="grid grid-cols-2 gap-3">
            <button
              type="button"
              onClick={() => {
                setRoutingChoice("direct");
                onChange({ nodeId: "" });
              }}
              className={`flex flex-col items-center gap-2 rounded-lg border-2 p-4 text-center transition-colors ${
                routingChoice === "direct"
                  ? "border-primary bg-primary/5"
                  : "border-border hover:border-primary/40"
              }`}
            >
              <Globe className="h-5 w-5" />
              <span className="text-xs font-medium">Direct</span>
              <span className="text-[10px] text-muted-foreground">
                NyxID proxies to endpoint
              </span>
            </button>
            <button
              type="button"
              onClick={() => setRoutingChoice("node")}
              className={`flex flex-col items-center gap-2 rounded-lg border-2 p-4 text-center transition-colors ${
                routingChoice === "node"
                  ? "border-primary bg-primary/5"
                  : "border-border hover:border-primary/40"
              }`}
            >
              <Server className="h-5 w-5" />
              <span className="text-xs font-medium">Via Node</span>
              <span className="text-[10px] text-muted-foreground">
                Route through credential node
              </span>
            </button>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            SSH services must be routed through a credential node.
          </p>
        )}

        {routingChoice === "node" && (
          <div className="space-y-1.5">
            <Label htmlFor="routing-node-select">Select Node</Label>
            <Select
              value={form.nodeId || undefined}
              onValueChange={(v) => onChange({ nodeId: v })}
            >
              <SelectTrigger id="routing-node-select">
                <SelectValue placeholder="Choose a node..." />
              </SelectTrigger>
              <SelectContent>
                {isLoading && (
                  <SelectItem value="_loading" disabled>
                    Loading nodes...
                  </SelectItem>
                )}
                {onlineNodes.map((node) => (
                  <SelectItem key={node.id} value={node.id}>
                    <span className="flex items-center gap-2">
                      <Server className="h-3.5 w-3.5" />
                      {node.name}
                    </span>
                  </SelectItem>
                ))}
                {!isLoading && onlineNodes.length === 0 && (
                  <SelectItem value="_none" disabled>
                    No online nodes
                  </SelectItem>
                )}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>

      <Button
        className="w-full"
        onClick={handleNext}
        disabled={routingChoice === "node" && !form.nodeId}
      >
        {routingChoice === "node"
          ? "Next: Node Setup"
          : "Next: Enter Credentials"}
      </Button>
    </div>
  );
}

function KeyForm({
  catalogEntry,
  form,
  onChange,
  onSubmit,
  onBack,
  isPending,
}: {
  readonly catalogEntry: CatalogEntry | null;
  readonly form: FormState;
  readonly onChange: (updates: Partial<FormState>) => void;
  readonly onSubmit: () => void;
  readonly onBack: () => void;
  readonly isPending: boolean;
}) {
  const isCustom = catalogEntry === null;
  const endpointEditable =
    isCustom || (catalogEntry?.requires_gateway_url ?? false);
  const requiresCredential = isCustom
    ? form.authMethod !== "none"
    : (catalogEntry?.auth_method ?? "bearer") !== "none";
  const requiresEndpoint = isCustom || (catalogEntry?.requires_gateway_url ?? false);
  const credentialMeta = getCredentialFieldMeta(
    form.authMethod,
    form.authKeyName,
  );

  return (
    <div className="space-y-4">
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3 w-3" />
        Back
      </button>

      {catalogEntry && (
        <div className="rounded-lg border border-border bg-muted/50 p-3">
          <p className="text-sm font-medium">{catalogEntry.name}</p>
          {catalogEntry.description && (
            <p className="text-xs text-muted-foreground">
              {catalogEntry.description}
            </p>
          )}
          {catalogEntry.api_key_url && (
            <a
              href={catalogEntry.api_key_url}
              target="_blank"
              rel="noopener noreferrer"
              className="mt-1 inline-flex items-center gap-1 text-xs text-primary hover:underline"
            >
              Get API key
              <ExternalLink className="h-3 w-3" />
            </a>
          )}
        </div>
      )}

      {catalogEntry?.api_key_instructions && (
        <p className="text-xs text-muted-foreground">
          {catalogEntry.api_key_instructions}
        </p>
      )}

      <div className="space-y-3">
        <div className="space-y-1.5">
          <Label htmlFor="add-key-label">Label <span className="text-destructive">*</span></Label>
          <Input
            id="add-key-label"
            placeholder={
              catalogEntry
                ? `e.g., ${catalogEntry.name} - Production`
                : "My API Key"
            }
            value={form.label}
            onChange={(e) => onChange({ label: e.target.value })}
          />
          <p className="text-[11px] text-muted-foreground">
            Give it a name you'll recognize. The proxy slug is auto-generated
            from this.
          </p>
        </div>

        {form.authMethod === "token_exchange" &&
        (catalogEntry?.token_exchange_credential_fields?.length ?? 0) > 0 ? (
          (() => {
            const fields = catalogEntry?.token_exchange_credential_fields ?? [];
            const values = parseTokenExchangeCredential(form.credential);
            return (
              <>
                {fields.map((field) => (
                  <div key={field.name} className="space-y-1.5">
                    <Label htmlFor={`add-key-te-${field.name}`}>
                      {field.label}
                      {requiresCredential && (
                        <span className="text-destructive"> *</span>
                      )}
                    </Label>
                    <Input
                      id={`add-key-te-${field.name}`}
                      type={field.secret ? "password" : "text"}
                      placeholder={
                        field.placeholder ?? `Enter ${field.label.toLowerCase()}`
                      }
                      value={values[field.name] ?? ""}
                      onChange={(e) =>
                        onChange({
                          credential: composeTokenExchangeCredential({
                            ...values,
                            [field.name]: e.target.value,
                          }),
                        })
                      }
                    />
                    {field.secret && (
                      <p className="text-[11px] text-muted-foreground">
                        Stored encrypted. NyxID exchanges this server-side
                        for an access token and caches it -- you never have
                        to refresh tokens, and the secret never leaves NyxID.
                      </p>
                    )}
                  </div>
                ))}
              </>
            );
          })()
        ) : (
          <div className="space-y-1.5">
            <Label htmlFor="add-key-credential">
              {credentialMeta.label}
              {requiresCredential && <span className="text-destructive"> *</span>}
            </Label>
            <Input
              id="add-key-credential"
              type={requiresCredential ? "password" : "text"}
              placeholder={
                requiresCredential
                  ? credentialMeta.placeholder
                  : "No credential required for this service"
              }
              value={requiresCredential ? form.credential : ""}
              onChange={(e) => onChange({ credential: e.target.value })}
              disabled={!requiresCredential}
              className={!requiresCredential ? "bg-muted text-muted-foreground" : ""}
            />
            {!requiresCredential && (
              <p className="text-[11px] text-muted-foreground">
                This service can be used without storing a user credential in NyxID.
              </p>
            )}
            {requiresCredential && form.authMethod === "body" && (
              <p className="text-[11px] text-muted-foreground">
                NyxID injects this value into the request JSON body under the
                field name below.
              </p>
            )}
            {requiresCredential && form.authMethod === "bot_bearer" && (
              <p className="text-[11px] text-muted-foreground">
                Sent as <code className="font-mono">Authorization: Bot &lt;token&gt;</code>.
              </p>
            )}
          </div>
        )}

        <div className="space-y-1.5">
          <Label htmlFor="add-key-endpoint">
            Endpoint URL{" "}
            {(isCustom || catalogEntry?.requires_gateway_url) && (
              <span className="text-destructive">*</span>
            )}
          </Label>
          <Input
            id="add-key-endpoint"
            placeholder="https://api.example.com/v1"
            value={form.endpointUrl}
            onChange={(e) => onChange({ endpointUrl: e.target.value })}
            readOnly={!endpointEditable}
            className={
              endpointEditable
                ? ""
                : "bg-muted text-muted-foreground cursor-default"
            }
          />
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="add-key-openapi-spec">
            OpenAPI spec URL <span className="text-muted-foreground">(optional)</span>
          </Label>
          <Input
            id="add-key-openapi-spec"
            placeholder="https://api.example.com/openapi.json"
            value={form.openapiSpecUrl}
            onChange={(e) => onChange({ openapiSpecUrl: e.target.value })}
            type="url"
          />
          <p className="text-[11px] text-muted-foreground">
            When set, AI agents discover concrete operations from the spec instead
            of being limited to a single generic proxy tool.
          </p>
        </div>

        {(isCustom || catalogEntry?.auth_method !== "none") && (
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="add-key-auth-method">Auth Method</Label>
              <Select
                value={form.authMethod}
                onValueChange={(v) =>
                  onChange({
                    authMethod: v,
                    authKeyName: AUTH_METHOD_DEFAULTS[v] ?? "Authorization",
                  })
                }
              >
                <SelectTrigger id="add-key-auth-method">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="bearer">Bearer</SelectItem>
                  <SelectItem value="header">Header</SelectItem>
                  <SelectItem value="query">Query Parameter</SelectItem>
                  <SelectItem value="path">Path Prefix</SelectItem>
                  <SelectItem value="basic">Basic Auth</SelectItem>
                  <SelectItem value="body">JSON Body Injection</SelectItem>
                  <SelectItem value="bot_bearer">Bot Token (Discord)</SelectItem>
                  <SelectItem value="token_exchange">
                    Token Exchange (Lark / OAuth client_credentials)
                  </SelectItem>
                  <SelectItem value="oauth2">OAuth 2.0</SelectItem>
                  <SelectItem value="oidc">OIDC</SelectItem>
                  <SelectItem value="none">None</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {shouldShowAuthKeyName(form.authMethod) && (
              <div className="space-y-1.5">
                <Label htmlFor="add-key-auth-key">
                  {getAuthKeyNameLabel(form.authMethod)}
                </Label>
                <Input
                  id="add-key-auth-key"
                  placeholder={
                    form.authMethod === "body" ? "app_secret" : "Authorization"
                  }
                  value={form.authKeyName}
                  onChange={(e) => onChange({ authKeyName: e.target.value })}
                />
              </div>
            )}
          </div>
        )}
      </div>

      <Button
        className="w-full"
        onClick={onSubmit}
        disabled={
          isPending ||
          !form.label.trim() ||
          (requiresCredential && !form.credential.trim()) ||
          (requiresEndpoint && !form.endpointUrl.trim())
        }
      >
        {isPending ? "Creating..." : "Create Service"}
      </Button>
    </div>
  );
}

function NodeSetupStep({
  catalogEntry,
  form,
  onChange,
  onSubmit,
  onBack,
  isPending,
}: {
  readonly catalogEntry: CatalogEntry | null;
  readonly form: FormState;
  readonly onChange: (updates: Partial<FormState>) => void;
  readonly onSubmit: () => void;
  readonly onBack: () => void;
  readonly isPending: boolean;
}) {
  const isCustom = catalogEntry === null;
  const previewSlug =
    isCustom && form.label.trim()
      ? form.label
          .trim()
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, "-")
          .replace(/^-|-$/g, "")
          .slice(0, 40)
      : "";
  const slug = catalogEntry?.slug ?? (previewSlug || "<slug>");
  const isSsh =
    catalogEntry?.service_type === "ssh" || form.serviceType === "ssh";

  return (
    <div className="space-y-4">
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3 w-3" />
        Back
      </button>

      <div className="space-y-1.5">
        <Label htmlFor="node-label">
          Label <span className="text-destructive">*</span>
        </Label>
        <Input
          id="node-label"
          placeholder={catalogEntry?.name ?? "My Service"}
          value={form.label}
          onChange={(e) => onChange({ label: e.target.value })}
        />
      </div>

      {isCustom && (
        <div className="space-y-3">
          {isSsh ? (
            <div className="space-y-3">
              <div className="grid grid-cols-2 gap-3">
                <div className="space-y-1.5">
                  <Label htmlFor="node-ssh-host">SSH Host</Label>
                  <Input
                    id="node-ssh-host"
                    placeholder="192.168.1.100 (optional, configured on node)"
                    value={form.sshHost}
                    onChange={(e) => onChange({ sshHost: e.target.value })}
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="node-ssh-port">SSH Port</Label>
                  <Input
                    id="node-ssh-port"
                    type="number"
                    placeholder="22"
                    value={form.sshPort}
                    onChange={(e) => onChange({ sshPort: e.target.value })}
                  />
                </div>
              </div>

              <div className="space-y-1.5">
                <Label htmlFor="node-ssh-cert-auth">Certificate Auth</Label>
                <Select
                  value={form.sshCertificateAuth ? "enabled" : "disabled"}
                  onValueChange={(v) =>
                    onChange({ sshCertificateAuth: v === "enabled" })
                  }
                >
                  <SelectTrigger id="node-ssh-cert-auth">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="enabled">Enabled</SelectItem>
                    <SelectItem value="disabled">
                      Disabled (transport only)
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>

              {form.sshCertificateAuth && (
                <>
                  <div className="space-y-1.5">
                    <Label htmlFor="node-ssh-principals">
                      Allowed Principals{" "}
                      <span className="text-destructive">*</span>
                    </Label>
                    <Input
                      id="node-ssh-principals"
                      placeholder="ubuntu, deploy"
                      value={form.sshPrincipals}
                      onChange={(e) =>
                        onChange({ sshPrincipals: e.target.value })
                      }
                    />
                    <p className="text-[11px] text-muted-foreground">
                      Comma-separated Unix usernames for certificate login
                    </p>
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="node-ssh-ttl">
                      Certificate TTL (minutes)
                    </Label>
                    <Input
                      id="node-ssh-ttl"
                      type="number"
                      placeholder="30"
                      value={form.sshCertificateTtlMinutes}
                      onChange={(e) =>
                        onChange({ sshCertificateTtlMinutes: e.target.value })
                      }
                    />
                    <p className="text-[11px] text-muted-foreground">
                      15-60 minutes. Shorter is more secure.
                    </p>
                  </div>
                </>
              )}
            </div>
          ) : (
            <>
              <div className="space-y-1.5">
                <Label htmlFor="node-endpoint">Endpoint URL</Label>
                <Input
                  id="node-endpoint"
                  placeholder="https://api.example.com/v1"
                  value={form.endpointUrl}
                  onChange={(e) => onChange({ endpointUrl: e.target.value })}
                />
                <p className="text-[11px] text-muted-foreground">
                  The target URL configured on your node agent
                </p>
              </div>
            </>
          )}

          {!isSsh && (
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <Label htmlFor="node-auth-method">Auth Method</Label>
                <Select
                  value={form.authMethod}
                  onValueChange={(v) =>
                    onChange({
                      authMethod: v,
                      authKeyName: AUTH_METHOD_DEFAULTS[v] ?? "Authorization",
                    })
                  }
                >
                  <SelectTrigger id="node-auth-method">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="bearer">Bearer</SelectItem>
                    <SelectItem value="header">Header</SelectItem>
                    <SelectItem value="query">Query Parameter</SelectItem>
                    <SelectItem value="path">Path Prefix</SelectItem>
                    <SelectItem value="basic">Basic Auth</SelectItem>
                    <SelectItem value="body">JSON Body Injection</SelectItem>
                    <SelectItem value="bot_bearer">Bot Token (Discord)</SelectItem>
                    <SelectItem value="oauth2">OAuth 2.0</SelectItem>
                    <SelectItem value="oidc">OIDC</SelectItem>
                    <SelectItem value="none">None</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {shouldShowAuthKeyName(form.authMethod) && (
                <div className="space-y-1.5">
                  <Label htmlFor="node-auth-key">
                    {getAuthKeyNameLabel(form.authMethod)}
                  </Label>
                  <Input
                    id="node-auth-key"
                    placeholder={
                      form.authMethod === "body" ? "app_secret" : "Authorization"
                    }
                    value={form.authKeyName}
                    onChange={(e) =>
                      onChange({ authKeyName: e.target.value })
                    }
                  />
                </div>
              )}
            </div>
          )}
        </div>
      )}

      <div className="rounded-lg border border-border bg-muted/50 p-4 space-y-3">
        <div className="flex items-center gap-2">
          <Terminal className="h-4 w-4 text-primary" />
          <p className="text-sm font-medium">Node Setup Instructions</p>
        </div>

        {isSsh ? (
          <div className="space-y-2">
            <p className="text-xs text-muted-foreground">
              The SSH service will be created with certificate-based
              authentication. After creation, full setup instructions (CA key,
              sshd_config, node agent setup) will be available on the service
              detail page.
            </p>
          </div>
        ) : (
          <div className="space-y-3">
            <p className="text-xs text-muted-foreground">
              Run this on your node to auto-setup credentials. It detects the
              service type and guides you through the right flow:
            </p>
            <CopyableCode>
              {`nyxid node credentials setup --service ${slug || "<slug>"}`}
            </CopyableCode>
            {isCustom && (
              <p className="text-[11px] text-muted-foreground">
                The exact service slug will be shown on the service detail page
                after creation. Update the{" "}
                <code className="text-[10px]">--service</code> flag accordingly.
              </p>
            )}
            {catalogEntry?.api_key_url && (
              <a
                href={catalogEntry.api_key_url}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
              >
                Get API key
                <ExternalLink className="h-3 w-3" />
              </a>
            )}
          </div>
        )}
      </div>

      <Button
        className="w-full"
        onClick={onSubmit}
        disabled={
          isPending ||
          !form.label.trim() ||
          (isCustom &&
            isSsh &&
            form.sshCertificateAuth &&
            !form.sshPrincipals.trim())
        }
      >
        {isPending ? "Creating..." : "Create Service"}
      </Button>
    </div>
  );
}

/**
 * Parse the free-form "additional scopes" textbox into a trimmed, deduped list.
 * Accepts comma-, space-, or newline-separated values. Mirrors the CLI's
 * `--scope` flag and the backend's `parse_additional_scopes` splitter so that
 * input is forgiving regardless of how the user pastes scopes from docs.
 */
function parseAdditionalScopes(raw: string): readonly string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const piece of raw.split(/[,\s]+/)) {
    const trimmed = piece.trim();
    if (trimmed && !seen.has(trimmed)) {
      seen.add(trimmed);
      out.push(trimmed);
    }
  }
  return out;
}

function OAuthStep({
  catalogEntry,
  ensureKey,
  onBack,
}: {
  readonly catalogEntry: CatalogEntry;
  readonly ensureKey: () => Promise<KeyInfo>;
  readonly onBack: () => void;
}) {
  const initiateOAuth = useInitiateOAuth();
  const [error, setError] = useState<string | null>(null);
  const [scopeInput, setScopeInput] = useState("");

  async function handleConnect() {
    if (!catalogEntry.provider_config_id) return;
    setError(null);
    try {
      const key = await ensureKey();
      const additionalScopes = parseAdditionalScopes(scopeInput);
      const response = await initiateOAuth.mutateAsync({
        providerId: catalogEntry.provider_config_id,
        redirectPath: `/keys/${key.id}`,
        additionalScopes,
      });
      hardRedirect(response.authorization_url);
    } catch (err) {
      const message =
        err instanceof ApiError ? err.message : "Failed to start OAuth flow";
      setError(message);
    }
  }

  return (
    <div className="space-y-4">
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3 w-3" />
        Back
      </button>

      <div className="rounded-lg border border-border bg-muted/50 p-3">
        <p className="text-sm font-medium">{catalogEntry.name}</p>
        {catalogEntry.description && (
          <p className="text-xs text-muted-foreground">
            {catalogEntry.description}
          </p>
        )}
      </div>

      <p className="text-sm text-muted-foreground">
        This service uses OAuth to authenticate. Click the button below to
        connect your account.
      </p>

      <div className="space-y-1.5">
        <Label htmlFor="oauth-additional-scopes" className="text-xs">
          Additional scopes (optional)
        </Label>
        <Input
          id="oauth-additional-scopes"
          value={scopeInput}
          onChange={(e) => setScopeInput(e.target.value)}
          placeholder="e.g. contact:contact.base:readonly, contact:department.base:readonly"
          autoComplete="off"
          spellCheck={false}
        />
        <p className="text-xs text-muted-foreground">
          Comma- or space-separated. Merged with the provider's default scopes.
          The upstream provider decides whether to grant them.
        </p>
      </div>

      {error && (
        <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
          {error}
        </div>
      )}

      <Button
        className="w-full"
        onClick={() => void handleConnect()}
        disabled={initiateOAuth.isPending}
      >
        {initiateOAuth.isPending ? (
          <>
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            Connecting...
          </>
        ) : (
          <>
            <ExternalLink className="mr-2 h-4 w-4" />
            Connect with {catalogEntry.name}
          </>
        )}
      </Button>
    </div>
  );
}

type DeviceFlowStep =
  | "configure"
  | "requesting"
  | "show_code"
  | "success"
  | "error";

function DeviceCodeStep({
  catalogEntry,
  ensureKey,
  onBack,
  onComplete,
}: {
  readonly catalogEntry: CatalogEntry;
  readonly ensureKey: () => Promise<KeyInfo>;
  readonly onBack: () => void;
  readonly onComplete: (keyId: string) => void;
}) {
  const [flowStep, setFlowStep] = useState<DeviceFlowStep>("configure");
  const [userCode, setUserCode] = useState("");
  const [verificationUri, setVerificationUri] = useState("");
  const [errorMessage, setErrorMessage] = useState("");
  const [secondsRemaining, setSecondsRemaining] = useState(0);
  const [createdKeyId, setCreatedKeyId] = useState<string | null>(null);
  const [scopeInput, setScopeInput] = useState("");

  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const countdownTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const isMountedRef = useRef(true);

  const initiateMutation = useInitiateDeviceCode();
  const pollMutation = usePollDeviceCode();

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
      if (pollTimerRef.current) {
        clearTimeout(pollTimerRef.current);
        pollTimerRef.current = null;
      }
      if (countdownTimerRef.current) {
        clearInterval(countdownTimerRef.current);
        countdownTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (flowStep !== "show_code" || secondsRemaining <= 0) return;

    countdownTimerRef.current = setInterval(() => {
      if (!isMountedRef.current) return;
      setSecondsRemaining((prev) => {
        if (prev <= 1) {
          if (countdownTimerRef.current) {
            clearInterval(countdownTimerRef.current);
            countdownTimerRef.current = null;
          }
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => {
      if (countdownTimerRef.current) {
        clearInterval(countdownTimerRef.current);
        countdownTimerRef.current = null;
      }
    };
  }, [flowStep, secondsRemaining]);

  // `schedulePoll` is recursive: while polling, the success handler's
  // "pending" / "slow_down" branches (and the error handler) re-schedule the
  // next poll by calling the same function. React Compiler's
  // `react-hooks/immutability` rule flags a direct self-reference inside the
  // callback body as an access-before-initialization, so we route the
  // recursive calls through a ref that is synced to the current callback
  // after every render. This keeps the existing polling behavior identical
  // while satisfying the linter.
  type SchedulePoll = (
    providerId: string,
    state: string,
    interval: number,
  ) => void;
  const schedulePollRef = useRef<SchedulePoll | null>(null);

  const schedulePoll = useCallback<SchedulePoll>(
    (providerId, state, interval) => {
      if (!isMountedRef.current) return;

      pollTimerRef.current = setTimeout(() => {
        if (!isMountedRef.current) return;

        pollMutation.mutate(
          { providerId, state },
          {
            onSuccess: (data: DeviceCodePollResponse) => {
              if (!isMountedRef.current) return;
              switch (data.status) {
                case "pending":
                  schedulePollRef.current?.(
                    providerId,
                    state,
                    data.interval ?? interval,
                  );
                  break;
                case "slow_down":
                  schedulePollRef.current?.(
                    providerId,
                    state,
                    data.interval ?? interval + 5,
                  );
                  break;
                case "complete":
                  setFlowStep("success");
                  break;
                case "expired":
                  setErrorMessage("Authentication expired. Please try again.");
                  setFlowStep("error");
                  break;
                case "denied":
                  setErrorMessage("Authentication was denied.");
                  setFlowStep("error");
                  break;
              }
            },
            onError: () => {
              if (isMountedRef.current) {
                schedulePollRef.current?.(providerId, state, interval);
              }
            },
          },
        );
      }, interval * 1000);
    },
    [pollMutation],
  );

  // Keep the ref in sync with the latest `schedulePoll` identity so recursive
  // calls always hit the current callback instance.
  useEffect(() => {
    schedulePollRef.current = schedulePoll;
  }, [schedulePoll]);

  async function handleInitiate() {
    if (!catalogEntry.provider_config_id) {
      setErrorMessage("Provider configuration not available");
      setFlowStep("error");
      return;
    }
    setErrorMessage("");
    setFlowStep("requesting");
    try {
      const key = await ensureKey();
      if (!isMountedRef.current) return;
      setCreatedKeyId(key.id);
      // Only forward additional scopes for formats that accept them. OpenAI
      // device-code providers reject a `scope` parameter at the backend.
      const additionalScopes =
        catalogEntry.device_code_format === "openai"
          ? []
          : parseAdditionalScopes(scopeInput);
      const response = await initiateMutation.mutateAsync({
        providerId: catalogEntry.provider_config_id,
        additionalScopes,
      });
      if (!isMountedRef.current) return;

      setUserCode(response.user_code);
      setVerificationUri(response.verification_uri);
      setSecondsRemaining(response.expires_in);
      setFlowStep("show_code");

      schedulePoll(
        catalogEntry.provider_config_id,
        response.state,
        response.interval,
      );
    } catch (error) {
      if (!isMountedRef.current) return;
      if (error instanceof ApiError) {
        setErrorMessage(error.message);
      } else {
        setErrorMessage("Failed to request device code");
      }
      setFlowStep("error");
    }
  }

  function handleCopyCode() {
    void copyToClipboard(userCode).then(() => {
      toast.success("Code copied to clipboard");
    });
  }

  function handleRetry() {
    if (pollTimerRef.current) {
      clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
    if (countdownTimerRef.current) {
      clearInterval(countdownTimerRef.current);
      countdownTimerRef.current = null;
    }
    setUserCode("");
    setVerificationUri("");
    setErrorMessage("");
    setSecondsRemaining(0);
    void handleInitiate();
  }

  function formatTime(seconds: number): string {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${String(mins)}:${String(secs).padStart(2, "0")}`;
  }

  if (flowStep === "configure") {
    // OpenAI-format device-code providers (e.g. Codex) do not accept a
    // `scope` parameter -- scopes are baked into the client registration.
    // Hide the scope input for those and show a short note instead, so the
    // user never enters something the backend will reject.
    const supportsAdditionalScopes =
      catalogEntry.device_code_format !== "openai";

    return (
      <div className="space-y-4">
        <button
          type="button"
          onClick={onBack}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-3 w-3" />
          Back
        </button>

        <div className="rounded-lg border border-border bg-muted/50 p-3">
          <p className="text-sm font-medium">{catalogEntry.name}</p>
          {catalogEntry.description && (
            <p className="text-xs text-muted-foreground">
              {catalogEntry.description}
            </p>
          )}
        </div>

        <p className="text-sm text-muted-foreground">
          This service uses a device code to authenticate. Click continue to
          request a code.
        </p>

        {supportsAdditionalScopes ? (
          <div className="space-y-1.5">
            <Label htmlFor="device-additional-scopes" className="text-xs">
              Additional scopes (optional)
            </Label>
            <Input
              id="device-additional-scopes"
              value={scopeInput}
              onChange={(e) => setScopeInput(e.target.value)}
              placeholder="e.g. repo,read:org"
              autoComplete="off"
              spellCheck={false}
            />
            <p className="text-xs text-muted-foreground">
              Comma- or space-separated. Merged with the provider's default
              scopes. The upstream provider decides whether to grant them.
            </p>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">
            This provider does not accept additional scopes -- they are fixed
            by the upstream client registration.
          </p>
        )}

        <Button className="w-full" onClick={() => void handleInitiate()}>
          Continue
        </Button>
      </div>
    );
  }

  if (flowStep === "requesting") {
    return (
      <div className="space-y-4">
        <button
          type="button"
          onClick={onBack}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-3 w-3" />
          Back
        </button>
        <div className="flex flex-col items-center gap-3 py-8">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
          <p className="text-sm text-muted-foreground">
            Requesting code from {catalogEntry.name}...
          </p>
        </div>
      </div>
    );
  }

  if (flowStep === "success") {
    return (
      <div className="space-y-4">
        <div className="flex flex-col items-center gap-3 py-4">
          <CheckCircle2 className="h-10 w-10 text-success" />
          <p className="text-sm text-center text-muted-foreground">
            Your {catalogEntry.name} account has been connected successfully.
          </p>
        </div>
        <Button
          className="w-full"
          onClick={() => {
            if (createdKeyId) {
              onComplete(createdKeyId);
            }
          }}
          disabled={!createdKeyId}
        >
          Done
        </Button>
      </div>
    );
  }

  if (flowStep === "error") {
    return (
      <div className="space-y-4">
        <button
          type="button"
          onClick={onBack}
          className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-3 w-3" />
          Back
        </button>
        <div className="flex flex-col items-center gap-3 py-4">
          <AlertCircle className="h-10 w-10 text-destructive" />
          <p className="text-sm text-destructive text-center">{errorMessage}</p>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" className="flex-1" onClick={onBack}>
            Cancel
          </Button>
          <Button className="flex-1" onClick={handleRetry}>
            Try Again
          </Button>
        </div>
      </div>
    );
  }

  // show_code step
  return (
    <div className="space-y-4">
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3 w-3" />
        Back
      </button>

      <div className="flex flex-col items-center gap-3 rounded-lg border-2 border-dashed border-primary/30 bg-primary/5 p-6">
        <p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
          Your code
        </p>
        <div className="flex items-center gap-3">
          <code className="text-3xl font-bold tracking-[0.3em] font-mono text-primary">
            {userCode}
          </code>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={handleCopyCode}
            className="h-8 w-8 p-0"
            title="Copy code"
          >
            <Copy className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <div className="flex justify-center">
        <Button type="button" variant="default" size="lg" asChild>
          <a href={verificationUri} target="_blank" rel="noopener noreferrer">
            <ExternalLink className="mr-2 h-4 w-4" />
            Open {catalogEntry.name} Authentication
          </a>
        </Button>
      </div>

      <div className="rounded-md bg-muted p-3 text-sm text-muted-foreground">
        <ol className="list-decimal list-inside space-y-1">
          <li>Click the link above to open the authentication page</li>
          <li>Enter the code shown above</li>
          <li>Sign in with your account</li>
        </ol>
      </div>

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <div className="flex items-center gap-2">
          <Loader2 className="h-3 w-3 animate-spin" />
          <span>Waiting for authentication...</span>
        </div>
        {secondsRemaining > 0 && (
          <span>Expires in {formatTime(secondsRemaining)}</span>
        )}
      </div>
    </div>
  );
}

function OAuthCredentialsStep({
  catalogEntry,
  onBack,
  onComplete,
}: {
  readonly catalogEntry: CatalogEntry;
  readonly onBack: () => void;
  readonly onComplete: () => void;
}) {
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [error, setError] = useState<string | null>(null);
  const setCredentials = useSetProviderCredentials();

  async function handleSave() {
    if (!catalogEntry.provider_config_id) return;
    setError(null);

    try {
      await setCredentials.mutateAsync({
        providerId: catalogEntry.provider_config_id,
        client_id: clientId.trim(),
        client_secret: clientSecret.trim() || undefined,
      });
      onComplete();
    } catch (err) {
      const message =
        err instanceof ApiError
          ? err.message
          : "Failed to save OAuth credentials";
      setError(message);
    }
  }

  return (
    <div className="space-y-4">
      <button
        type="button"
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-3 w-3" />
        Back
      </button>

      <div className="rounded-lg border border-border bg-muted/50 p-3">
        <p className="text-sm font-medium">{catalogEntry.name}</p>
        <p className="text-xs text-muted-foreground">
          This service requires your own OAuth app credentials.
        </p>
      </div>

      {catalogEntry.documentation_url && (
        <a
          href={catalogEntry.documentation_url}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
        >
          How to create an OAuth app
          <ExternalLink className="h-3 w-3" />
        </a>
      )}

      {error && (
        <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
          {error}
        </div>
      )}

      <div className="space-y-3">
        <div className="space-y-1.5">
          <Label htmlFor="oauth-client-id">
            Client ID <span className="text-destructive">*</span>
          </Label>
          <Input
            id="oauth-client-id"
            placeholder="Your OAuth app Client ID"
            value={clientId}
            onChange={(e) => setClientId(e.target.value)}
            autoComplete="off"
          />
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="oauth-client-secret">Client Secret</Label>
          <Input
            id="oauth-client-secret"
            type="password"
            placeholder="Your OAuth app Client Secret (optional for public clients)"
            value={clientSecret}
            onChange={(e) => setClientSecret(e.target.value)}
            autoComplete="off"
          />
        </div>
      </div>

      <Button
        className="w-full"
        onClick={() => void handleSave()}
        disabled={setCredentials.isPending || !clientId.trim()}
      >
        {setCredentials.isPending ? "Saving..." : "Continue to Authentication"}
      </Button>
    </div>
  );
}

export function AddKeyDialog({
  open,
  onOpenChange,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const createKey = useCreateKey();
  const [step, setStep] = useState<WizardStep>("catalog");
  const [selectedEntry, setSelectedEntry] = useState<CatalogEntry | null>(null);
  const [form, setForm] = useState<FormState>(INITIAL_FORM);
  const [authKey, setAuthKey] = useState<KeyInfo | null>(null);

  function resetWizard() {
    setStep("catalog");
    setSelectedEntry(null);
    setForm(INITIAL_FORM);
    setAuthKey(null);
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      resetWizard();
    }
    onOpenChange(next);
  }

  function handleSelectCatalog(entry: CatalogEntry) {
    setSelectedEntry(entry);
    setAuthKey(null);
    setForm({
      ...INITIAL_FORM,
      label: entry.name,
      endpointUrl: entry.base_url,
      authMethod: entry.auth_method ?? "bearer",
      authKeyName: entry.auth_key_name ?? "Authorization",
    });
    setStep("routing");
  }

  function handleSelectCustom() {
    setSelectedEntry(null);
    setAuthKey(null);
    setForm(INITIAL_FORM);
    setStep("routing");
  }

  function handleSelectCustomSsh() {
    setSelectedEntry(null);
    setAuthKey(null);
    setForm({
      ...INITIAL_FORM,
      serviceType: "ssh",
      authMethod: "none",
      authKeyName: "",
      sshCertificateAuth: true,
      sshPort: "22",
      sshCertificateTtlMinutes: "30",
    });
    setStep("routing");
  }

  function handleRoutingDirect() {
    if (!selectedEntry) {
      setStep("form");
      return;
    }

    const needsUserCreds =
      selectedEntry.credential_mode === "user" ||
      selectedEntry.credential_mode === "both";

    if (
      selectedEntry.provider_type === "oauth2" &&
      selectedEntry.provider_config_id
    ) {
      setStep(needsUserCreds ? "oauth_credentials" : "oauth");
      return;
    }

    if (
      selectedEntry.provider_type === "device_code" &&
      selectedEntry.provider_config_id
    ) {
      setStep(needsUserCreds ? "oauth_credentials" : "device_code");
      return;
    }

    setStep("form");
  }

  function handleRoutingViaNode() {
    setStep("node_setup");
  }

  function handleCredentialsSaved() {
    if (!selectedEntry) return;
    if (selectedEntry.provider_type === "device_code") {
      setStep("device_code");
    } else {
      setStep("oauth");
    }
  }

  function handleFormChange(updates: Partial<FormState>) {
    setAuthKey(null);
    setForm((prev) => ({ ...prev, ...updates }));
  }

  function buildCatalogKeyParams() {
    if (!selectedEntry) {
      throw new Error("Catalog entry is required for this flow");
    }

    return {
      label: form.label,
      service_slug: selectedEntry.slug,
      ...(form.endpointUrl.trim()
        ? { endpoint_url: form.endpointUrl.trim() }
        : {}),
      ...(form.authMethod !== (selectedEntry.auth_method ?? "bearer")
        ? { auth_method: form.authMethod }
        : {}),
      ...(form.authKeyName !== (selectedEntry.auth_key_name ?? "Authorization")
        ? { auth_key_name: form.authKeyName }
        : {}),
      ...(form.nodeId.trim() ? { node_id: form.nodeId.trim() } : {}),
      ...(form.openapiSpecUrl.trim()
        ? { openapi_spec_url: form.openapiSpecUrl.trim() }
        : {}),
    };
  }

  async function ensureAuthKey(): Promise<KeyInfo> {
    if (authKey) {
      return authKey;
    }

    const key = await createKey.mutateAsync(buildCatalogKeyParams());
    setAuthKey(key);
    return key;
  }

  function handleAuthComplete(keyId: string) {
    toast.success("Service connected");
    handleOpenChange(false);
    void navigate({ to: "/keys/$keyId", params: { keyId } });
  }

  function handleFormSubmit() {
    const specUrl = form.openapiSpecUrl.trim();
    const params = selectedEntry
      ? {
          credential: form.credential,
          label: form.label,
          service_slug: selectedEntry.slug,
          ...(form.endpointUrl.trim()
            ? { endpoint_url: form.endpointUrl.trim() }
            : {}),
          ...(form.authMethod !== (selectedEntry.auth_method ?? "bearer")
            ? { auth_method: form.authMethod }
            : {}),
          ...(form.authKeyName !==
          (selectedEntry.auth_key_name ?? "Authorization")
            ? { auth_key_name: form.authKeyName }
            : {}),
          ...(specUrl ? { openapi_spec_url: specUrl } : {}),
        }
      : {
          credential: form.credential,
          label: form.label,
          endpoint_url: form.endpointUrl.trim(),
          auth_method: form.authMethod,
          auth_key_name: form.authKeyName,
          ...(specUrl ? { openapi_spec_url: specUrl } : {}),
        };

    createKey.mutate(params, {
      onSuccess: (key) => {
        toast.success("Key created");
        handleOpenChange(false);
        void navigate({ to: "/keys/$keyId", params: { keyId: key.id } });
      },
      onError: (err) => {
        const message =
          err instanceof ApiError ? err.message : "Failed to create key";
        toast.error(message);
      },
    });
  }

  function handleNodeSetupSubmit() {
    // Node routing: create the service directly. No OAuth/credentials needed --
    // the node agent handles auth locally via `nyxid node credentials setup`.
    const isSshCustom = !selectedEntry && form.serviceType === "ssh";
    const params = selectedEntry
      ? {
          label: form.label,
          service_slug: selectedEntry.slug,
          node_id: form.nodeId,
          service_type: selectedEntry.service_type,
        }
      : isSshCustom
        ? {
            label: form.label,
            node_id: form.nodeId,
            service_type: "ssh" as const,
            ssh_host: form.sshHost.trim(),
            ssh_port: Number(form.sshPort) || 22,
            ssh_certificate_auth: form.sshCertificateAuth,
            ssh_principals: form.sshPrincipals.trim(),
            ssh_certificate_ttl_minutes:
              Number(form.sshCertificateTtlMinutes) || 30,
          }
        : {
            label: form.label,
            endpoint_url: form.endpointUrl.trim() || undefined,
            auth_method: form.authMethod,
            auth_key_name: form.authKeyName,
            node_id: form.nodeId,
            service_type: form.serviceType,
            ...(form.openapiSpecUrl.trim()
              ? { openapi_spec_url: form.openapiSpecUrl.trim() }
              : {}),
          };

    createKey.mutate(params, {
      onSuccess: (key) => {
        toast.success("Service created");
        handleOpenChange(false);
        void navigate({ to: "/keys/$keyId", params: { keyId: key.id } });
      },
      onError: (err) => {
        const message =
          err instanceof ApiError ? err.message : "Failed to create service";
        toast.error(message);
      },
    });
  }

  function dialogTitle(): string {
    switch (step) {
      case "catalog":
        return "Add AI Service";
      case "routing":
        return "Configure Routing";
      case "node_setup":
        return "Node Setup";
      case "oauth_credentials":
        return `Setup ${selectedEntry?.name ?? "Service"} Credentials`;
      case "oauth":
        return `Connect to ${selectedEntry?.name ?? "Service"}`;
      case "device_code":
        return `Connect to ${selectedEntry?.name ?? "Service"}`;
      default:
        return "Configure Service";
    }
  }

  function dialogDescription(): string {
    switch (step) {
      case "catalog":
        return "Pick from the catalog or create a custom endpoint.";
      case "routing":
        return "Choose how requests reach the endpoint.";
      case "node_setup":
        return "Configure credentials on your node agent.";
      case "oauth_credentials":
        return `Enter your OAuth app credentials for ${selectedEntry?.name ?? "the service"}.`;
      case "oauth":
        return `Authenticate with ${selectedEntry?.name ?? "the service"} via OAuth.`;
      case "device_code":
        return `Authenticate with ${selectedEntry?.name ?? "the service"} using a device code.`;
      default:
        return selectedEntry
          ? `Set up your ${selectedEntry.name} credentials.`
          : "Configure your custom endpoint and credentials.";
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{dialogTitle()}</DialogTitle>
          <DialogDescription>{dialogDescription()}</DialogDescription>
        </DialogHeader>

        {step === "catalog" && (
          <CatalogGrid
            onSelect={handleSelectCatalog}
            onCustom={handleSelectCustom}
            onCustomSsh={handleSelectCustomSsh}
          />
        )}

        {step === "routing" && (
          <RoutingStep
            catalogEntry={selectedEntry}
            form={form}
            onChange={handleFormChange}
            onDirect={handleRoutingDirect}
            onViaNode={handleRoutingViaNode}
            onBack={() => setStep("catalog")}
            isSshOnly={
              selectedEntry?.service_type === "ssh" ||
              form.serviceType === "ssh"
            }
          />
        )}

        {step === "form" && (
          <KeyForm
            catalogEntry={selectedEntry}
            form={form}
            onChange={handleFormChange}
            onSubmit={handleFormSubmit}
            onBack={() => setStep("routing")}
            isPending={createKey.isPending}
          />
        )}

        {step === "node_setup" && (
          <NodeSetupStep
            catalogEntry={selectedEntry}
            form={form}
            onChange={handleFormChange}
            onSubmit={handleNodeSetupSubmit}
            onBack={() => setStep("routing")}
            isPending={createKey.isPending}
          />
        )}

        {step === "oauth_credentials" && selectedEntry && (
          <OAuthCredentialsStep
            catalogEntry={selectedEntry}
            onBack={() => setStep("routing")}
            onComplete={handleCredentialsSaved}
          />
        )}

        {step === "oauth" && selectedEntry && (
          <OAuthStep
            catalogEntry={selectedEntry}
            ensureKey={ensureAuthKey}
            onBack={() =>
              setStep(
                selectedEntry.credential_mode === "user" ||
                  selectedEntry.credential_mode === "both"
                  ? "oauth_credentials"
                  : form.nodeId.trim()
                    ? "node_setup"
                    : "routing",
              )
            }
          />
        )}

        {step === "device_code" && selectedEntry && (
          <DeviceCodeStep
            catalogEntry={selectedEntry}
            ensureKey={ensureAuthKey}
            onBack={() =>
              setStep(
                selectedEntry.credential_mode === "user" ||
                  selectedEntry.credential_mode === "both"
                  ? "oauth_credentials"
                  : form.nodeId.trim()
                    ? "node_setup"
                    : "routing",
              )
            }
            onComplete={handleAuthComplete}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}
