import { useState, useEffect, useRef, useCallback } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useCatalog, useCreateKey } from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import { useOrgs } from "@/hooks/use-orgs";
import {
  useInitiateOAuth,
  useInitiateDeviceCode,
  usePollDeviceCode,
} from "@/hooks/use-providers";
import { ApiError, api } from "@/lib/api-client";
import { hardRedirect } from "@/lib/navigation";
import { copyToClipboard } from "@/lib/utils";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { OAuthCallbackGuidance } from "@/components/shared/twitter-oauth-guidance";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
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
  Building2,
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
  aws_sigv4: "",
  none: "",
};

// AWS SigV4 credential is a JSON object with these fields, stored on
// the backend as the encrypted `credential` blob. `session_token` is
// optional for callers using STS temporary credentials (Codex review
// REC 9). Default region+service cover the AWS Cost Explorer common
// case; users with other AWS services can override.
interface AwsSigv4Fields {
  readonly access_key_id: string;
  readonly secret_access_key: string;
  readonly region: string;
  readonly service: string;
  readonly session_token: string;
}

const AWS_SIGV4_DEFAULTS: AwsSigv4Fields = {
  access_key_id: "",
  secret_access_key: "",
  region: "us-east-1",
  service: "ce",
  session_token: "",
};

function parseAwsSigv4Credential(credential: string): AwsSigv4Fields {
  if (!credential) return AWS_SIGV4_DEFAULTS;
  try {
    const parsed = JSON.parse(credential);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return {
        access_key_id: typeof parsed.access_key_id === "string" ? parsed.access_key_id : "",
        secret_access_key:
          typeof parsed.secret_access_key === "string" ? parsed.secret_access_key : "",
        region: typeof parsed.region === "string" ? parsed.region : AWS_SIGV4_DEFAULTS.region,
        service: typeof parsed.service === "string" ? parsed.service : AWS_SIGV4_DEFAULTS.service,
        session_token:
          typeof parsed.session_token === "string" ? parsed.session_token : "",
      };
    }
  } catch {
    // Fall through to defaults on parse error.
  }
  return AWS_SIGV4_DEFAULTS;
}

function composeAwsSigv4Credential(fields: AwsSigv4Fields): string {
  const trimmedToken = fields.session_token.trim();
  const anyPresent =
    fields.access_key_id.trim() ||
    fields.secret_access_key.trim() ||
    fields.region.trim() !== AWS_SIGV4_DEFAULTS.region ||
    fields.service.trim() !== AWS_SIGV4_DEFAULTS.service ||
    trimmedToken;
  if (!anyPresent) return "";
  const payload: Record<string, string> = {
    access_key_id: fields.access_key_id.trim(),
    secret_access_key: fields.secret_access_key.trim(),
    region: fields.region.trim() || AWS_SIGV4_DEFAULTS.region,
    service: fields.service.trim() || AWS_SIGV4_DEFAULTS.service,
  };
  // Only include `session_token` when the user typed one — STS
  // temporary credentials need it, long-lived IAM users don't.
  if (trimmedToken) {
    payload.session_token = trimmedToken;
  }
  return JSON.stringify(payload);
}

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
  if (authMethod === "aws_sigv4") {
    return {
      label: "AWS Credentials",
      placeholder: "Use the structured fields below",
    };
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
    authMethod !== "token_exchange" &&
    authMethod !== "aws_sigv4"
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
      <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-xs leading-relaxed">
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
          <Skeleton key={i} className="h-[7.5rem] rounded-lg" />
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

      <div className="grid grid-cols-2 gap-3 md:max-h-[380px] md:overflow-y-auto md:pr-1 sm:grid-cols-3">
        <button
          type="button"
          onClick={onCustom}
          className="flex min-h-[7.5rem] flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-border p-4 text-center transition-colors duration-300 hover:border-white/[0.15] hover:bg-accent/40"
        >
          <Globe className="h-5 w-5 text-muted-foreground" />
          <span className="text-xs font-medium">Custom Endpoint</span>
        </button>

        <button
          type="button"
          onClick={onCustomSsh}
          className="flex min-h-[7.5rem] flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-border p-4 text-center transition-colors duration-300 hover:border-white/[0.15] hover:bg-accent/40"
        >
          <Terminal className="h-5 w-5 text-muted-foreground" />
          <span className="text-xs font-medium">Custom SSH</span>
        </button>

        {filtered?.map((entry) => (
          <button
            key={entry.slug}
            type="button"
            onClick={() => onSelect(entry)}
            className="flex min-h-[7.5rem] flex-col items-start gap-1.5 rounded-lg border border-border p-4 text-left transition-colors duration-300 hover:border-white/[0.15] hover:bg-accent/40"
          >
            <span className="line-clamp-1 w-full text-[12px] font-medium">
              {entry.name}
            </span>
            <span className="line-clamp-2 w-full text-[11px] leading-snug text-muted-foreground">
              {entry.description ?? entry.base_url}
            </span>
            <div className="mt-auto flex w-full flex-wrap gap-1 pt-1">
              {entry.service_type === "ssh" && (
                <Badge variant="secondary" className="text-[10px]">
                  SSH
                </Badge>
              )}
              {entry.requires_gateway_url && (
                <Badge variant="secondary" className="text-[10px]">
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
            </div>
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
          <p className="text-[12px] font-medium">{catalogEntry.name}</p>
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
              className={`flex flex-col items-center gap-2 rounded-lg border-2 p-4 text-center transition-colors duration-300 ${
                routingChoice === "direct"
                  ? "border-primary bg-primary/5"
                  : "border-border hover:border-white/[0.15]"
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
              className={`flex flex-col items-center gap-2 rounded-lg border-2 p-4 text-center transition-colors duration-300 ${
                routingChoice === "node"
                  ? "border-primary bg-primary/5"
                  : "border-border hover:border-white/[0.15]"
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
          <p className="text-[12px] text-muted-foreground">
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
        variant="primary"
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
          <p className="text-[12px] font-medium">{catalogEntry.name}</p>
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
        ) : form.authMethod === "aws_sigv4" ? (
          (() => {
            const fields = parseAwsSigv4Credential(form.credential);
            const update = (patch: Partial<AwsSigv4Fields>) =>
              onChange({
                credential: composeAwsSigv4Credential({ ...fields, ...patch }),
              });
            return (
              <>
                <div className="space-y-1.5">
                  <Label htmlFor="add-key-aws-akid">
                    Access Key ID
                    {requiresCredential && <span className="text-destructive"> *</span>}
                  </Label>
                  <Input
                    id="add-key-aws-akid"
                    placeholder="AKIA..."
                    value={fields.access_key_id}
                    onChange={(e) => update({ access_key_id: e.target.value })}
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="add-key-aws-secret">
                    Secret Access Key
                    {requiresCredential && <span className="text-destructive"> *</span>}
                  </Label>
                  <Input
                    id="add-key-aws-secret"
                    type="password"
                    placeholder="40-character secret"
                    value={fields.secret_access_key}
                    onChange={(e) => update({ secret_access_key: e.target.value })}
                  />
                  <p className="text-[11px] text-muted-foreground">
                    Stored encrypted. The IAM policy attached to this key
                    enforces read-only — NyxID never elevates it.
                  </p>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1.5">
                    <Label htmlFor="add-key-aws-region">Region</Label>
                    <Input
                      id="add-key-aws-region"
                      placeholder="us-east-1"
                      value={fields.region}
                      onChange={(e) => update({ region: e.target.value })}
                    />
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="add-key-aws-service">Service</Label>
                    <Input
                      id="add-key-aws-service"
                      placeholder="ce"
                      value={fields.service}
                      onChange={(e) => update({ service: e.target.value })}
                    />
                  </div>
                </div>
                <p className="text-[11px] text-muted-foreground">
                  Cost Explorer is single-region (us-east-1, service=ce).
                  For other AWS services, change `service` to match
                  (e.g. `s3`, `dynamodb`).
                </p>
                <div className="space-y-1.5">
                  <Label htmlFor="add-key-aws-session-token">
                    Session Token <span className="text-text-tertiary">(optional, for STS temporary credentials)</span>
                  </Label>
                  <Input
                    id="add-key-aws-session-token"
                    type="password"
                    placeholder="Leave blank for long-lived IAM user credentials"
                    value={fields.session_token}
                    onChange={(e) => update({ session_token: e.target.value })}
                  />
                </div>
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

      <DialogFooter>
        <Button
          variant="primary"
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
      </DialogFooter>
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
          <p className="text-[12px] font-medium">Node Setup Instructions</p>
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

      <DialogFooter>
        <Button
          variant="primary"
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
      </DialogFooter>
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

async function cleanupPendingAuthKey(
  key: KeyInfo | null,
  options: { readonly protectExistingKey?: boolean } = {},
) {
  if (options.protectExistingKey) return;
  if (key?.status !== "pending_auth") return;
  try {
    await api.delete<void>(
      `/keys/${encodeURIComponent(key.id)}?only_if_pending=true`,
    );
  } catch {
    // Best effort only. The detail page still exposes Delete Service
    // for any pending placeholder that survives this cleanup.
  }
}

function OAuthStep({
  catalogEntry,
  ensureKey,
  onKeyCleared,
  onBack,
  targetOrgId,
  reconnectMode,
}: {
  readonly catalogEntry: CatalogEntry;
  readonly ensureKey: () => Promise<KeyInfo>;
  readonly onKeyCleared: () => void;
  readonly onBack: () => void;
  /** When set, initiate the OAuth flow under this org's scope. */
  readonly targetOrgId: string | null;
  readonly reconnectMode: boolean;
}) {
  const initiateOAuth = useInitiateOAuth();
  const [error, setError] = useState<string | null>(null);
  const [scopeInput, setScopeInput] = useState("");

  async function handleConnect() {
    if (!catalogEntry.provider_config_id) return;
    setError(null);
    let key: KeyInfo | null = null;
    try {
      key = await ensureKey();
      const additionalScopes = parseAdditionalScopes(scopeInput);
      const response = await initiateOAuth.mutateAsync({
        providerId: catalogEntry.provider_config_id,
        redirectPath: `/keys/${key.id}`,
        additionalScopes,
        // Multi-connection: thread the placeholder's id so the OAuth
        // callback writes the resulting tokens straight onto THIS
        // `UserApiKey` (via its `connection_id`) instead of taking
        // the legacy `user_provider_tokens` path. Without this, a
        // second add to the same provider would leave the freshly-
        // minted placeholder stuck in `pending_auth` while the token
        // landed on the legacy row.
        keyId: key.id,
        ...(targetOrgId ? { targetOrgId } : {}),
      });
      hardRedirect(response.authorization_url);
    } catch (err) {
      await cleanupPendingAuthKey(key, { protectExistingKey: reconnectMode });
      if (!reconnectMode) {
        onKeyCleared();
      }
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
        <p className="text-[12px] font-medium">{catalogEntry.name}</p>
        {catalogEntry.description && (
          <p className="text-xs text-muted-foreground">
            {catalogEntry.description}
          </p>
        )}
      </div>

      <p className="text-[12px] text-muted-foreground">
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
        <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
          {error}
        </div>
      )}

      <Button
        variant="primary"
        className="w-full"
        onClick={() => void handleConnect()}
        disabled={initiateOAuth.isPending}
      >
        {initiateOAuth.isPending ? (
          <>
            <ButtonIcon><Loader2 className="h-3 w-3 animate-spin" /></ButtonIcon>
            Connecting...
          </>
        ) : (
          <>
            <ButtonIcon><ExternalLink className="h-4 w-4" /></ButtonIcon>
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
  onKeyCleared,
  // Renamed locally so the in-component `onBack` (defined below) can wrap it
  // with placeholder cleanup. Every existing `onClick={onBack}` in the JSX
  // below now goes through the wrapper without further changes (NyxID#706).
  onBack: parentOnBack,
  onComplete,
  targetOrgId,
  reconnectMode,
}: {
  readonly catalogEntry: CatalogEntry;
  readonly ensureKey: () => Promise<KeyInfo>;
  readonly onKeyCleared: () => void;
  readonly onBack: () => void;
  readonly onComplete: (keyId: string) => void;
  /** When set, initiate the device-code flow under this org's scope. */
  readonly targetOrgId: string | null;
  readonly reconnectMode: boolean;
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
  // Mirrors `createdKeyId` for the unmount-time DELETE. The closure inside
  // useEffect's cleanup can only see values captured at mount, so a ref is the
  // only way to reach the *current* placeholder id at teardown (NyxID#706).
  const createdKeyIdRef = useRef<string | null>(null);

  const initiateMutation = useInitiateDeviceCode();
  const pollMutation = usePollDeviceCode();

  useEffect(() => {
    createdKeyIdRef.current = createdKeyId;
  }, [createdKeyId]);

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
      // Fire-and-forget cleanup of any still-pending placeholder. The
      // `?only_if_pending=true` guard makes this a no-op if the user happened
      // to complete authorization in the same tick (the `complete` poll
      // branch clears the ref so the typical success path skips it entirely).
      const orphan = reconnectMode ? null : createdKeyIdRef.current;
      if (orphan) {
        createdKeyIdRef.current = null;
        void api
          .delete<void>(
            `/keys/${encodeURIComponent(orphan)}?only_if_pending=true`,
          )
          .catch(() => {
            // Best effort only.
          });
      }
    };
  }, [reconnectMode]);

  // User-driven exit paths (Back / Cancel buttons, `expired`/`denied` poll
  // outcomes). Awaits the DELETE so the parent's `authKey` state is consistent
  // before the user can re-enter the step; the unmount cleanup is the
  // backstop for tab-close / dialog-X.
  //
  // Wrapped in `useCallback` so `schedulePoll` (which references it inside its
  // poll-success switch) doesn't see a fresh identity every render. The deps
  // intentionally exclude `createdKeyId` — we read the ref-mirrored value
  // first, so the callback stays stable across in-flight setState updates.
  const cleanupAndForgetKey = useCallback(async () => {
    const id = createdKeyIdRef.current;
    if (!id) return;
    createdKeyIdRef.current = null;
    if (isMountedRef.current) {
      setCreatedKeyId(null);
    }
    if (!reconnectMode) {
      onKeyCleared();
    }
    if (reconnectMode) return;
    try {
      await api.delete<void>(
        `/keys/${encodeURIComponent(id)}?only_if_pending=true`,
      );
    } catch {
      // Best effort only — the lazy reconciler in the backend will catch
      // any orphan that survives this call.
    }
  }, [onKeyCleared, reconnectMode]);

  // Wrapper around the parent's `onBack` that first cleans up the
  // `pending_auth` placeholder. Named `onBack` so the existing JSX
  // (`onClick={onBack}`) picks it up without further edits.
  function onBack() {
    void cleanupAndForgetKey();
    parentOnBack();
  }

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
                  // Suppress unmount cleanup — placeholder is now `active`.
                  createdKeyIdRef.current = null;
                  setFlowStep("success");
                  break;
                case "expired":
                  // Tear down the placeholder so a "Try Again" mints a fresh
                  // one instead of reusing a row whose `OAuthState` the
                  // backend already deleted (NyxID#706).
                  void cleanupAndForgetKey();
                  setErrorMessage("Authentication expired. Please try again.");
                  setFlowStep("error");
                  break;
                case "denied":
                  void cleanupAndForgetKey();
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
    // `cleanupAndForgetKey` is wrapped in `useCallback` above with stable
    // deps (`onKeyCleared`), so `schedulePoll` stays stable as long as the
    // parent doesn't swap that prop. Listed here to satisfy
    // react-hooks/exhaustive-deps now that the poll-success switch invokes
    // it (NyxID#706).
    [pollMutation, cleanupAndForgetKey],
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
    let key: KeyInfo | null = null;
    try {
      key = await ensureKey();
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
        // Multi-connection: same rationale as the OAuth step — thread
        // the placeholder id so the device-code completion writes onto
        // THIS `UserApiKey` instead of the legacy `user_provider_tokens`
        // row. Required for `nyxid service add codex` to actually mint
        // a distinct second connection.
        keyId: key.id,
        ...(targetOrgId ? { targetOrgId } : {}),
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
      await cleanupPendingAuthKey(key, { protectExistingKey: reconnectMode });
      if (!reconnectMode) {
        onKeyCleared();
      }
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
          <p className="text-[12px] font-medium">{catalogEntry.name}</p>
          {catalogEntry.description && (
            <p className="text-xs text-muted-foreground">
              {catalogEntry.description}
            </p>
          )}
        </div>

        <p className="text-[12px] text-muted-foreground">
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

        <Button variant="primary" className="w-full" onClick={() => void handleInitiate()}>
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
          <p className="text-[12px] text-muted-foreground">
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
          <CheckCircle2 className="h-8 w-8 text-success" />
          <p className="text-[12px] text-center text-muted-foreground">
            Your {catalogEntry.name} account has been connected successfully.
          </p>
        </div>
        <Button
          variant="primary"
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
          <AlertCircle className="h-8 w-8 text-destructive" />
          <p className="text-[12px] text-destructive text-center">{errorMessage}</p>
        </div>
        <div className="flex justify-end gap-2">
          <Button variant="outline" className="flex-1" onClick={onBack}>
            Cancel
          </Button>
          <Button variant="primary" className="flex-1" onClick={handleRetry}>
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
            <ButtonIcon><ExternalLink className="h-4 w-4" /></ButtonIcon>
            Open {catalogEntry.name} Authentication
          </a>
        </Button>
      </div>

      <div className="rounded-lg bg-muted p-3 text-[12px] text-muted-foreground">
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
  /**
   * Fired after the user enters Custom App credentials. Carries the
   * trimmed plaintext up to the parent so the eventual `POST /keys`
   * for this add can include them as `oauth_client_id` /
   * `oauth_client_secret` (multi-connection BYO write path).
   *
   * Important — this step does NOT call `PUT /providers/{id}/credentials`.
   * That endpoint writes to the single-row-per-`(user, provider)`
   * `user_provider_credentials` table. If a user already has a legacy
   * single-connection (`connection_id: null`) credential for this
   * provider, a PUT here would overwrite its Custom App secret with
   * the new one — silently breaking the legacy connection's refresh
   * path (`oauth_flow::refresh_oauth_token` reads from
   * `user_provider_credentials`). Multi-connection refresh reads BYO
   * directly off the `UserApiKey`, so the legacy table doesn't need
   * to be touched at all for the new add. The legacy PUT endpoint
   * remains available for callers that explicitly want to manage the
   * shared row (admin tooling, the legacy `connect provider` page).
   */
  readonly onComplete: (clientId: string, clientSecret: string) => void;
}) {
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [error, setError] = useState<string | null>(null);

  function handleSave() {
    if (!catalogEntry.provider_config_id) return;
    setError(null);
    const trimmedId = clientId.trim();
    const trimmedSecret = clientSecret.trim();
    onComplete(trimmedId, trimmedSecret);
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
        <p className="text-[12px] font-medium">{catalogEntry.name}</p>
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

      {/* Authorization-code OAuth flows redirect through NyxID's
          callback, so the user must register that URL in their provider
          app. Device-code BYO providers reach this same step but never
          use a redirect URI, so skip the callback for them. */}
      {catalogEntry.provider_type === "oauth2" && (
        <OAuthCallbackGuidance slug={catalogEntry.slug} />
      )}

      {error && (
        <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
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
        variant="primary"
        className="w-full"
        onClick={handleSave}
        disabled={!clientId.trim()}
      >
        Continue to Authentication
      </Button>
    </div>
  );
}

/**
 * Small owner selector rendered at the top of `AddKeyDialog`. Only surfaces
 * orgs where the caller is an admin (via `OrgScopeSelect`). The selected
 * org id is threaded into the create mutation via `target_org_id`, which
 * writes the resulting UserService under that org so every admin of the
 * org can manage it.
 */
function OwnerPicker({
  value,
  onChange,
}: {
  readonly value: string | null;
  readonly onChange: (orgId: string | null) => void;
}) {
  const { data: orgs } = useOrgs();
  // Hide the picker entirely when the caller has no admin orgs -- a
  // Personal-only select is noise in that case.
  const hasAdminOrg = (orgs ?? []).some((o) => o.your_role === "admin");
  if (!hasAdminOrg) return null;
  return (
    <div className="rounded-lg border border-border bg-muted/30 px-3 py-2">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
          <Building2 className="h-3.5 w-3.5" />
          Owner
        </div>
        <div className="w-[220px]">
          <OrgScopeSelect value={value} onChange={onChange} label="Owner" />
        </div>
      </div>
      <p className="mt-1 text-[11px] text-muted-foreground">
        Org-owned services are shared with every admin of that organization
        and can be proxied by its members.
      </p>
    </div>
  );
}

export function AddKeyDialog({
  open,
  onOpenChange,
  prefillSlug,
  reconnectKey,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /**
   * When set, the dialog opens with this catalog slug already
   * selected (skips the catalog picker). Used by the cli-pair
   * fallback handoff for auth flavors the remote-pairing path
   * can't finish — without this the user would land on the
   * generic catalog grid and have to hunt for the right entry.
   */
  readonly prefillSlug?: string;
  readonly reconnectKey?: KeyInfo | null;
}) {
  const navigate = useNavigate();
  const createKey = useCreateKey();
  const { data: catalogEntries } = useCatalog();
  const [step, setStep] = useState<WizardStep>("catalog");
  const [selectedEntry, setSelectedEntry] = useState<CatalogEntry | null>(null);
  const [form, setForm] = useState<FormState>(INITIAL_FORM);
  const [authKey, setAuthKey] = useState<KeyInfo | null>(null);
  const [targetOrgId, setTargetOrgId] = useState<string | null>(null);
  const isReconnect = reconnectKey !== undefined && reconnectKey !== null;
  // Multi-connection BYO: hold the user-typed Custom App credentials
  // from the `oauth_credentials` step so they can ride along on the
  // subsequent `POST /keys`. Without this, the dashboard would only
  // PUT them to the legacy `user_provider_credentials` row — which
  // can hold a single Custom App per (user, provider) — leaving the
  // new `UserApiKey.user_oauth_client_*_encrypted` empty. Refresh
  // would then fail at ~2h because the per-connection refresh path
  // has no client_id to use. See `designs/multi-connection-custom-
  // app-credentials.md` §6.1.
  const [byoOAuthClientId, setByoOAuthClientId] = useState<string | null>(null);
  const [byoOAuthClientSecret, setByoOAuthClientSecret] = useState<string | null>(null);
  // Guards `prefillSlug` against running its auto-select more than
  // once per dialog open. Without this, re-renders would re-select
  // the catalog entry and snap the user back to the routing step
  // if they tried to navigate backwards.
  const appliedPrefillRef = useRef<string | null>(null);

  function resetWizard() {
    setStep("catalog");
    setSelectedEntry(null);
    setForm(INITIAL_FORM);
    setAuthKey(null);
    setTargetOrgId(null);
    setByoOAuthClientId(null);
    setByoOAuthClientSecret(null);
    appliedPrefillRef.current = null;
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

  // Auto-select from `prefillSlug` once the catalog resolves. Only
  // fires on initial open (tracked via `appliedPrefillRef`) so a
  // user who navigates back to the catalog step can pick something
  // else. Silently no-ops if the slug isn't in the catalog — the
  // catalog step will render and the user can pick manually.
  useEffect(() => {
    if (!open || !reconnectKey || !catalogEntries) return;
    if (appliedPrefillRef.current === `reconnect:${reconnectKey.id}`) return;
    const slug = reconnectKey.catalog_service_slug ?? reconnectKey.slug;
    const match = catalogEntries.find((e) => e.slug === slug);
    if (!match) return;
    appliedPrefillRef.current = `reconnect:${reconnectKey.id}`;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- reconnect prefill is guarded by appliedPrefillRef to run once per reconnect key
    setSelectedEntry(match);
    setAuthKey(reconnectKey);
    setTargetOrgId(
      reconnectKey.credential_source?.type === "org" &&
        reconnectKey.credential_source.role === "admin"
        ? reconnectKey.credential_source.org_id
        : null,
    );
    setForm({
      ...INITIAL_FORM,
      label: reconnectKey.label || match.name,
      endpointUrl: reconnectKey.endpoint_url || match.base_url,
      authMethod: reconnectKey.auth_method || match.auth_method || "bearer",
      authKeyName:
        reconnectKey.auth_key_name || match.auth_key_name || "Authorization",
    });
    setStep(match.provider_type === "device_code" ? "device_code" : "oauth");
  }, [open, reconnectKey, catalogEntries]);

  useEffect(() => {
    if (!open || !prefillSlug || !catalogEntries) return;
    if (isReconnect) return;
    if (appliedPrefillRef.current === prefillSlug) return;
    const match = catalogEntries.find((e) => e.slug === prefillSlug);
    if (!match) return;
    appliedPrefillRef.current = prefillSlug;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- prefill selection is guarded to run once per slug
    handleSelectCatalog(match);
  }, [open, prefillSlug, catalogEntries, isReconnect]);

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
      ...(targetOrgId ? { target_org_id: targetOrgId } : {}),
      // Multi-connection BYO: include the user-typed Custom App
      // credentials in the same `POST /keys` so the new `UserApiKey`
      // carries its own encrypted copy. Either both halves or
      // neither — the backend rejects an unpaired submission.
      // Captured by `OAuthCredentialsStep` and stored on the parent
      // before this builder runs.
      ...(byoOAuthClientId && byoOAuthClientSecret
        ? {
            oauth_client_id: byoOAuthClientId,
            oauth_client_secret: byoOAuthClientSecret,
          }
        : {}),
    };
  }

  async function ensureAuthKey(): Promise<KeyInfo> {
    if (reconnectKey) {
      return reconnectKey;
    }
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
    const orgParam = targetOrgId ? { target_org_id: targetOrgId } : {};
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
          ...orgParam,
        }
      : {
          credential: form.credential,
          label: form.label,
          endpoint_url: form.endpointUrl.trim(),
          auth_method: form.authMethod,
          auth_key_name: form.authKeyName,
          ...(specUrl ? { openapi_spec_url: specUrl } : {}),
          ...orgParam,
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
    const orgParam = targetOrgId ? { target_org_id: targetOrgId } : {};
    const params = selectedEntry
      ? {
          label: form.label,
          service_slug: selectedEntry.slug,
          node_id: form.nodeId,
          service_type: selectedEntry.service_type,
          ...orgParam,
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
            ...orgParam,
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
            ...orgParam,
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
        return isReconnect ? "Reconnect Service" : "Add AI Service";
      case "routing":
        return "Configure Routing";
      case "node_setup":
        return "Node Setup";
      case "oauth_credentials":
        return `Setup ${selectedEntry?.name ?? "Service"} Credentials`;
      case "oauth":
        return `${isReconnect ? "Reconnect" : "Connect"} to ${selectedEntry?.name ?? "Service"}`;
      case "device_code":
        return `${isReconnect ? "Reconnect" : "Connect"} to ${selectedEntry?.name ?? "Service"}`;
      default:
        return "Configure Service";
    }
  }

  function dialogDescription(): string {
    switch (step) {
      case "catalog":
        return isReconnect
          ? "Restart authentication for this existing service."
          : "Pick from the catalog or create a custom endpoint.";
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
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{dialogTitle()}</DialogTitle>
          <DialogDescription>{dialogDescription()}</DialogDescription>
        </DialogHeader>

        {step === "catalog" && (
          <OwnerPicker value={targetOrgId} onChange={setTargetOrgId} />
        )}

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
            onComplete={(clientId, clientSecret) => {
              // Multi-connection: cache the user-typed Custom App
              // credentials on the parent so the eventual `POST /keys`
              // for this add can include them as `oauth_client_id` /
              // `oauth_client_secret`. `OAuthCredentialsStep` itself
              // intentionally does NOT call
              // `PUT /providers/{id}/credentials` (see the doc on the
              // `onComplete` prop at line 1798) — that legacy endpoint
              // writes to the single-row-per-`(user, provider)`
              // `user_provider_credentials` table, so PUT-ing from a
              // new BYO add would silently overwrite any pre-existing
              // single-connection user's Custom App secret. Refresh
              // for multi-connection keys reads BYO straight off the
              // `UserApiKey`, so the legacy table is left untouched.
              setByoOAuthClientId(clientId);
              setByoOAuthClientSecret(clientSecret);
              handleCredentialsSaved();
            }}
          />
        )}

        {step === "oauth" && selectedEntry && (
          <OAuthStep
            catalogEntry={selectedEntry}
            ensureKey={ensureAuthKey}
            onKeyCleared={() => setAuthKey(null)}
            targetOrgId={targetOrgId}
            reconnectMode={isReconnect}
            onBack={() =>
              isReconnect
                ? handleOpenChange(false)
                : setStep(
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
            onKeyCleared={() => setAuthKey(null)}
            targetOrgId={targetOrgId}
            reconnectMode={isReconnect}
            onBack={() =>
              isReconnect
                ? handleOpenChange(false)
                : setStep(
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
