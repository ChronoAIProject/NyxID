// ai-key (service-add) pairing panel.
//
// Supports all three provider shapes mirroring
// `cli/src/wizard/assets/wizard.js`:
//   - api_key / bearer — inline credential input, single `POST /keys`
//   - oauth            — placeholder key + popup + poll-until-active
//   - device_code      — placeholder key + code display + poll
//
// OAuth/device-code sub-flows live in `./ai-key-auth-flows.tsx` to
// keep this file focused on catalog lookup + dispatch.

import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { useOrgs } from "@/hooks/use-orgs";
import { ApiError, api } from "@/lib/api-client";
import { Building2, ExternalLink } from "lucide-react";
import type { AiKeyPrefill } from "@/pages/cli-pair/types";
import { DeviceCodeFlow, OAuthFlow } from "./auth-flows";
import {
  reservePairingAction,
  withRewindOnError,
} from "@/pages/cli-pair/reserve-action";
import { CatalogGrid } from "./catalog-grid";

/**
 * Catalog response shape we care about. Fields beyond these are
 * ignored — we don't narrow further because the catalog endpoint
 * returns ~30 fields of metadata and most aren't relevant here.
 */
/**
 * Per-field schema for `token_exchange` catalog entries. Matches the
 * backend's `CredentialFieldSpec`. Multiple fields are collected into
 * a JSON-encoded blob and sent as a single `credential` string on
 * `POST /keys`.
 */
interface CredentialFieldSpec {
  readonly name: string;
  readonly label: string;
  readonly placeholder?: string;
  readonly secret?: boolean;
}

interface CatalogEntryShape {
  readonly slug: string;
  readonly name: string;
  readonly description?: string;
  readonly base_url: string;
  /**
   * Downstream credential injection mode — `bearer`, `header`, `query`,
   * `basic`, `bot_bearer`, etc. Does NOT decide the pairing flow; use
   * `provider_type` for that.
   */
  readonly auth_method: string;
  /**
   * How the user *obtains* the credential — `oauth2`, `device_code`,
   * `api_key`. This is what the wizard routes on.
   */
  readonly provider_type?: string;
  readonly service_type: string;
  readonly requires_credential: boolean;
  readonly requires_gateway_url: boolean;
  /** Required by OAuth / device-code sub-flows. */
  readonly provider_config_id?: string;
  /**
   * `system` / `user` / `both` — whether this provider uses the
   * NyxID-admin-registered OAuth app, the user's own OAuth app, or
   * accepts both. When `user` or `both`, the OAuth pre-step collects
   * client_id / client_secret before the popup.
   */
  readonly credential_mode?: string;
  /** Admin-provided docs URL shown inside the user-OAuth-app step. */
  readonly documentation_url?: string;
  /**
   * Multi-field credential schema. When present and non-empty, this
   * catalog entry uses the `token_exchange` flow — render one form
   * input per field, collect a JSON object, and send as the
   * `credential` string. Used by Lark, Feishu, etc.
   */
  readonly token_exchange_credential_fields?: readonly CredentialFieldSpec[];
  readonly icon_url?: string;
  readonly api_key_url?: string;
  readonly api_key_instructions?: string;
}

/** Shape of `POST /keys` response we care about. */
interface UserServiceCreated {
  readonly id: string;
  readonly slug: string;
  readonly label: string;
  readonly endpoint_url: string;
}

export interface AiKeyPairingSuccess {
  readonly kind: "ai-key";
  readonly service_id: string;
  readonly slug: string;
  readonly label: string;
}

interface AiKeyConfirmProps {
  readonly prefill: AiKeyPrefill;
  /**
   * Pairing record id. Used to cancel the pairing server-side when
   * the user navigates away to the main Keys page (unsupported flow
   * fallback). Without this, the CLI keeps polling until TTL even
   * though the browser has given up on completing here.
   */
  readonly pairingId: string;
  readonly onSuccess: (result: AiKeyPairingSuccess) => void;
  /**
   * Fired on selected-slug transitions. A non-empty slug means the user
   * has entered credential-form territory; an empty string means the slug
   * was reset and the catalog grid is visible again. Parent uses this to
   * keep the shell's "Step X of 3" header in sync. Purely cosmetic — the
   * pairing state machine doesn't care.
   */
  readonly onSlugPicked?: (slug: string) => void;
}

/**
 * Flow classification. Mirrors the local wizard's `authShape` in
 * `cli/src/wizard/assets/wizard.js` — which routes on
 * `provider_type`, NOT `auth_method`. `auth_method` is the downstream
 * credential-injection mode (bearer vs header vs query vs basic, etc.);
 * an `auth_method: "bearer"` OAuth provider like GitHub is still
 * fundamentally an OAuth flow, and an `auth_method: "header"` api-key
 * provider like xAI still needs the paste-a-key UI.
 */
type AuthShape =
  | "no-auth"
  | "token-exchange"
  | "api-key"
  | "oauth"
  | "device-code"
  | "other";

function classifyFlow(entry: CatalogEntryShape): AuthShape {
  // OAuth / device-code win even if the entry also declares a credential
  // schema (provider_type is the source of truth for the auth flow).
  const pt = (entry.provider_type ?? "").toLowerCase();
  if (pt === "oauth2") return "oauth";
  if (pt === "device_code") return "device-code";
  // No-auth services (public proxies, non-credential downstreams) —
  // we just POST /keys and move on. Same shape the local wizard
  // calls "no-auth" / "1-click connect".
  if (entry.requires_credential === false) return "no-auth";
  // Token-exchange providers (Lark, Feishu, etc.) collect multiple
  // fields (app_id + app_secret, etc.) that the backend combines into
  // a JSON-encoded credential string. Check BEFORE "api-key" because
  // token_exchange entries also set `requires_credential: true`.
  if (
    Array.isArray(entry.token_exchange_credential_fields) &&
    entry.token_exchange_credential_fields.length > 0
  ) {
    return "token-exchange";
  }
  if (entry.requires_credential) return "api-key";
  return "other";
}

export function AiKeyConfirm({
  prefill,
  pairingId,
  onSuccess,
  onSlugPicked,
}: AiKeyConfirmProps) {
  // Issue #414: when the CLI passes `--custom`, drop straight onto
  // the `__custom__` step so the user lands on the custom-service
  // form (skipping the catalog grid). The form pre-populates from
  // prefill.label / endpoint_url / auth_method / auth_key_name /
  // custom_slug / via_node and asks the user for whatever's still
  // missing (typically the credential).
  const initialSlug = prefill.custom
    ? "__custom__"
    : (prefill.slug ?? "");
  const [slug, setSlug] = useState(initialSlug);
  const [targetOrgId, setTargetOrgId] = useState<string | null>(
    prefill.org_id ?? null,
  );
  const trimmedSlug = slug.trim();

  // Signal up whenever the selected slug changes. Fires once on mount
  // if the CLI prefilled the slug, once more when the user clicks a
  // catalog card, and with "" when the user returns to the catalog grid.
  const lastNotifiedSlug = useRef<string | null>(trimmedSlug ? null : "");
  useEffect(() => {
    if (lastNotifiedSlug.current !== trimmedSlug) {
      lastNotifiedSlug.current = trimmedSlug;
      onSlugPicked?.(trimmedSlug);
    }
  }, [trimmedSlug, onSlugPicked]);

  const {
    data: entry,
    isLoading: entryLoading,
    error: entryError,
  } = useQuery({
    queryKey: ["cli-pair", "catalog", trimmedSlug],
    queryFn: async (): Promise<CatalogEntryShape> => {
      return api.get<CatalogEntryShape>(
        `/catalog/${encodeURIComponent(trimmedSlug)}`,
      );
    },
    // `__custom__` is the CatalogGrid sentinel for the "Custom /
    // self-hosted…" card. It is not a real catalog entry — skip the
    // fetch and render the CustomServiceForm below instead.
    enabled: Boolean(trimmedSlug) && trimmedSlug !== "__custom__",
  });

  const entryErrorMessage = (() => {
    if (!entryError) return null;
    if (entryError instanceof ApiError) return entryError.message;
    return `Couldn't load catalog entry "${trimmedSlug}".`;
  })();

  // When no slug is picked yet, we're effectively on "Step 1 · pick a
  // service" (the catalog grid below). Once the user clicks a card we
  // transition to "Step 2 · enter credential" via the per-entry form.
  // The step-label header that wraps this panel in the shell keeps
  // the numeric step tied to the pairing phase, so we swap the title
  // copy here to mirror the old vanilla wizard's two-step narrative.
  const onCatalogStep = !trimmedSlug && !entryErrorMessage;
  const onCustomStep = trimmedSlug === "__custom__";
  const title = onCatalogStep
    ? "Add an AI service"
    : onCustomStep
      ? "Custom / self-hosted service"
      : "Connect service";
  const subtitle = onCatalogStep
    ? "Pick a service to connect. Simple-bearer APIs (OpenAI, Anthropic, Gemini) land in the guided form. Anything else — self-hosted, OAuth, device code, custom endpoint — goes to the power-user form."
    : onCustomStep
      ? "For services not in the catalog — paste your own endpoint URL and credential."
      : `Your CLI wants to add ${trimmedSlug || "a service"} to NyxID. Confirm the details here.`;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">{title}</h2>
        <p className="text-[12px] text-muted-foreground">{subtitle}</p>
      </div>

      <OwnerPicker value={targetOrgId} onChange={setTargetOrgId} />

      {onCatalogStep ? (
        <CatalogGrid onSelect={setSlug} />
      ) : onCustomStep ? (
        // The "Custom / self-hosted" catalog card emits the sentinel
        // slug `__custom__` — NOT a real catalog entry. Render a
        // stand-alone form for endpoint URL + credential + auth
        // method without fetching the catalog. Mirrors
        // `appendCustomFormFields` in the old vanilla wizard.js
        // (cli/src/wizard/assets/wizard.js:~325-400).
        <CustomServiceForm
          prefill={prefill}
          targetOrgId={targetOrgId}
          pairingId={pairingId}
          onSuccess={onSuccess}
          onBack={() => {
            setSlug("");
          }}
        />
      ) : entryLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : entryErrorMessage ? (
        // A bad slug or missing catalog entry would otherwise strand
        // the user on an error screen with no way to recover. Render
        // the picker underneath the error so they can edit the slug
        // in place without re-running the CLI.
        <div className="flex flex-col gap-3">
          <ErrorLine message={entryErrorMessage} />
          <CatalogGrid onSelect={setSlug} />
        </div>
      ) : entry ? (
        <CatalogConfirmForm
          entry={entry}
          prefill={prefill}
          targetOrgId={targetOrgId}
          pairingId={pairingId}
          onSuccess={onSuccess}
        />
      ) : null}
    </div>
  );
}

function OwnerPicker({
  value,
  onChange,
}: {
  readonly value: string | null;
  readonly onChange: (orgId: string | null) => void;
}) {
  const { data: orgs } = useOrgs();
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

// ── custom / self-hosted form (no catalog entry) ───────────────────

// All auth methods the backend's `POST /keys` accepts for custom
// services. Mirrors the CLI's `--auth-method` choices in
// `cli/src/cli.rs`. Issue #414 added the previously-missing
// `bot_bearer` / `path` / `body` so the wizard can fully replace
// the scripted CLI path for via-node + custom services.
type CustomAuthMethod =
  | "bearer"
  | "header"
  | "query"
  | "path"
  | "basic"
  | "body"
  | "bot_bearer"
  | "none";

/** Sensible default auth-key-name per method. Mirrors the CLI's
 *  `default_auth_key_name` in `cli/src/commands/service.rs:1437`. */
function defaultAuthKeyName(method: CustomAuthMethod): string {
  switch (method) {
    case "header":
      return "X-API-Key";
    case "query":
      return "key";
    case "path":
      return "bot";
    case "body":
      return "app_secret";
    default:
      return "Authorization";
  }
}

/** Validates that an arbitrary CLI-supplied string is a known auth
 *  method. Falls back to "bearer" on unrecognized values so a
 *  future CLI release can't break older bundles. */
function coerceAuthMethod(raw: string | undefined): CustomAuthMethod {
  switch (raw) {
    case "bearer":
    case "header":
    case "query":
    case "path":
    case "basic":
    case "body":
    case "bot_bearer":
    case "none":
      return raw;
    default:
      return "bearer";
  }
}

/** Whether this auth method needs a separate "key name" input
 *  beside the credential. `bearer` / `bot_bearer` always use
 *  `Authorization`; `basic` is `Authorization: Basic <user:pass>`;
 *  `none` has no credential at all. The remaining methods (header,
 *  query, path, body) each inject the credential into a named
 *  field, so they need the input. */
function authMethodNeedsKeyName(method: CustomAuthMethod): boolean {
  return (
    method === "header"
    || method === "query"
    || method === "path"
    || method === "body"
  );
}

interface CustomServiceFormProps {
  readonly prefill: AiKeyPrefill;
  readonly targetOrgId: string | null;
  readonly pairingId: string;
  readonly onSuccess: (result: AiKeyPairingSuccess) => void;
  readonly onBack: () => void;
}

function RequiredMarker() {
  return <span aria-hidden="true" className="text-destructive ml-0.5">*</span>;
}

function CustomServiceForm({
  prefill,
  targetOrgId,
  pairingId,
  onSuccess,
  onBack,
}: CustomServiceFormProps) {
  // Issue #414: when the CLI passed `--auth-method`, `--auth-key-name`,
  // `--slug`, `--via-node`, etc. alongside `--custom`, those values
  // pre-populate the form and let the user submit with just the
  // credential filled in. Without prefill the form starts at the same
  // bearer / Authorization defaults as before.
  const [label, setLabel] = useState(prefill.label ?? "");
  const [endpointUrl, setEndpointUrl] = useState(prefill.endpoint_url ?? "");
  const [credential, setCredential] = useState("");
  const [authMethod, setAuthMethod] = useState<CustomAuthMethod>(
    coerceAuthMethod(prefill.auth_method),
  );
  const [authKeyName, setAuthKeyName] = useState(
    prefill.auth_key_name
      ?? defaultAuthKeyName(coerceAuthMethod(prefill.auth_method)),
  );
  const [slug, setSlug] = useState(prefill.custom_slug ?? "");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // CLI-bound node — the user already chose this with `--via-node`
  // at the command line; the SPA shows it as a read-only badge and
  // forwards it on submit so the backend marks the credential as
  // pushable to the bound node (existing flow at
  // `backend/src/handlers/keys.rs:611-632`).
  const viaNode = prefill.via_node?.trim() ?? "";

  const trimmedLabel = label.trim();
  const trimmedEndpoint = endpointUrl.trim();
  const trimmedCredential = credential.trim();
  const needsCredential = authMethod !== "none";
  const needsKeyName = authMethodNeedsKeyName(authMethod);
  const submitDisabled =
    loading ||
    !trimmedLabel ||
    !trimmedEndpoint ||
    (needsCredential && !trimmedCredential);
  const credentialLabel =
    authMethod === "bot_bearer"
      ? "Bot token"
      : authMethod === "basic"
        ? "user:pass"
        : authMethod === "body"
          ? `${authKeyName.trim() || defaultAuthKeyName(authMethod)} value`
          : "API key / credential";

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      const body: Record<string, unknown> = {
        label: trimmedLabel,
        endpoint_url: trimmedEndpoint,
        auth_method: authMethod,
      };
      if (needsCredential) body.credential = trimmedCredential;
      // For methods that target a named field (header / query /
      // path / body) keep the user's auth-key-name override.
      // bearer / bot_bearer / basic always use `Authorization`,
      // so we omit the field there to match the CLI's scripted
      // body shape (see `cli/src/commands/service.rs:340-341`).
      if (needsKeyName) {
        body.auth_key_name = authKeyName.trim() || defaultAuthKeyName(authMethod);
      }
      const trimmedSlug = slug.trim();
      if (trimmedSlug) body.slug = trimmedSlug;
      // Issue #414: forward the CLI-supplied node id so the backend
      // `POST /keys` handler creates a node-routed UserService and
      // pushes the credential to that node over the existing WS
      // channel. Falls through to direct routing when not set.
      if (viaNode) body.node_id = viaNode;
      if (targetOrgId) body.target_org_id = targetOrgId;

      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<UserServiceCreated>("/keys", body),
      );
      onSuccess({
        kind: "ai-key",
        service_id: res.id,
        slug: res.slug,
        label: res.label,
      });
    } catch (e) {
      setError(
        e instanceof ApiError ? e.message : (e as Error).message ?? "Failed.",
      );
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center">
            <Label htmlFor="pair-custom-label">Label</Label>
            <RequiredMarker />
          </div>
          <Input
            id="pair-custom-label"
            value={label}
            onChange={(e) => {
              setLabel(e.target.value);
            }}
            placeholder="e.g. My Self-hosted OpenAI Proxy"
            autoFocus
            aria-required="true"
          />
          <p className="text-xs text-muted-foreground">
            Shown everywhere in the CLI and web UI.
          </p>
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center">
            <Label htmlFor="pair-custom-endpoint">Endpoint URL</Label>
            <RequiredMarker />
          </div>
          <Input
            id="pair-custom-endpoint"
            value={endpointUrl}
            onChange={(e) => {
              setEndpointUrl(e.target.value);
            }}
            placeholder="https://api.example.com"
            aria-required="true"
          />
          <p className="text-xs text-muted-foreground">
            The base URL NyxID proxies requests to.
          </p>
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center">
            <Label htmlFor="pair-custom-auth-method">Auth method</Label>
            <RequiredMarker />
          </div>
          <select
            id="pair-custom-auth-method"
            value={authMethod}
            onChange={(e) => {
              const next = e.target.value as CustomAuthMethod;
              setAuthMethod(next);
              // Reset the key-name to the new method's default if the
              // user hadn't customized it yet — saves them re-typing
              // for every method switch. We only overwrite when the
              // current value matches some other method's default
              // (or is empty); user-typed values stay put.
              const knownDefaults: readonly string[] = [
                "Authorization",
                "X-API-Key",
                "key",
                "bot",
                "app_secret",
              ];
              if (!authKeyName.trim() || knownDefaults.includes(authKeyName.trim())) {
                setAuthKeyName(defaultAuthKeyName(next));
              }
            }}
            className="flex h-10 w-full rounded-lg border border-input bg-transparent px-[14px] py-2 text-[13px] text-foreground focus-visible:outline-none"
            aria-required="true"
          >
            <option value="bearer">bearer (Authorization: Bearer …)</option>
            <option value="bot_bearer">bot_bearer (Authorization: Bot …)</option>
            <option value="header">header (custom header)</option>
            <option value="query">query (?key=…)</option>
            <option value="path">path (path-prefix injection)</option>
            <option value="basic">basic (Authorization: Basic …)</option>
            <option value="body">body (JSON-body field injection)</option>
            <option value="none">none (no auth injection)</option>
          </select>
          <p className="text-xs text-muted-foreground">
            How NyxID attaches the credential to outgoing requests.
          </p>
        </div>

        {needsCredential ? (
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center">
              <Label htmlFor="pair-custom-credential">{credentialLabel}</Label>
              <RequiredMarker />
            </div>
            <Input
              id="pair-custom-credential"
              type="password"
              value={credential}
              onChange={(e) => {
                setCredential(e.target.value);
              }}
              placeholder={
                authMethod === "basic" ? "user:pass" : "sk-..."
              }
              autoFocus={Boolean(prefill.custom)}
              aria-required="true"
            />
            <p className="text-xs text-muted-foreground">
              Pasted once, encrypted at rest.
              {authMethod === "basic" ? " Format: user:pass." : ""}
            </p>
          </div>
        ) : null}

        {needsKeyName ? (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="pair-custom-auth-key-name">
              {authMethod === "header"
                ? "Header name"
                : authMethod === "query"
                  ? "Query parameter name"
                  : authMethod === "path"
                    ? "Path prefix segment"
                    : "Body field name"}
            </Label>
            <Input
              id="pair-custom-auth-key-name"
              value={authKeyName}
              onChange={(e) => {
                setAuthKeyName(e.target.value);
              }}
              placeholder={defaultAuthKeyName(authMethod)}
            />
          </div>
        ) : null}

        {viaNode ? (
          // Issue #414: surface the bound node so the user can see
          // which agent will receive the pushed credential. The node
          // id is locked at this point — to change it the user
          // re-runs the CLI with a different `--via-node`.
          <div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
            <p className="text-xs font-medium text-foreground">Routed via node</p>
            <code className="font-mono text-[11px] text-muted-foreground">
              {viaNode}
            </code>
            <p className="text-[11px] text-muted-foreground mt-1">
              Credential will be encrypted and pushed to this node over
              the existing WebSocket channel. NyxID never logs it.
            </p>
          </div>
        ) : null}

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="pair-custom-slug">Custom slug (optional)</Label>
          <Input
            id="pair-custom-slug"
            value={slug}
            onChange={(e) => {
              setSlug(e.target.value);
            }}
            placeholder="auto-generated from label"
          />
          <p className="text-xs text-muted-foreground">
            URL segment at <code>/proxy/s/&lt;slug&gt;/…</code>. Leave
            blank to let NyxID derive it from the label.
          </p>
        </div>
      </div>

      {error ? <ErrorLine message={error} /> : null}

      <div className="flex items-center justify-between gap-2">
        <Button variant="outline" onClick={onBack} disabled={loading}>
          ← Back
        </Button>
        <Button variant="primary" onClick={() => void submit()} disabled={submitDisabled}>
          {loading ? "Connecting…" : "Connect service"}
        </Button>
      </div>
    </div>
  );
}

// ── per-entry confirm: renders the right inputs for the auth method ─

interface CatalogConfirmFormProps {
  readonly entry: CatalogEntryShape;
  readonly prefill: AiKeyPrefill;
  readonly targetOrgId: string | null;
  readonly pairingId: string;
  readonly onSuccess: (result: AiKeyPairingSuccess) => void;
}

function CatalogConfirmForm({
  entry,
  prefill,
  targetOrgId,
  pairingId,
  onSuccess,
}: CatalogConfirmFormProps) {
  const shape = classifyFlow(entry);
  const [label, setLabel] = useState(prefill.label ?? entry.name);
  const [credential, setCredential] = useState("");
  const [endpointUrl, setEndpointUrl] = useState(prefill.endpoint_url ?? "");
  // Per-field state for token-exchange flows. Keyed by the spec
  // `name`; combined into a JSON-encoded `credential` string on
  // submit. One hook per entry so flipping between entries doesn't
  // leak values across providers.
  const [tokenFields, setTokenFields] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const viaNode = prefill.via_node?.trim() ?? "";
  // Set once the user clicks "Continue" for an OAuth / device-code
  // flow; drops the confirm UI and mounts the sub-flow panel. Reset
  // to `null` if the sub-flow reports cancel so the user can edit
  // label / retry.
  const [authFlowActive, setAuthFlowActive] = useState(false);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      const body: Record<string, unknown> = {
        service_slug: entry.slug,
        label,
      };
      if (shape === "token-exchange" && !viaNode) {
        // Multi-field credential: validate required-ness and JSON-
        // encode. Mirror wizard.js submit path at wizard.js:723-734.
        const fields = entry.token_exchange_credential_fields ?? [];
        const creds: Record<string, string> = {};
        for (const f of fields) {
          const val = tokenFields[f.name]?.trim();
          if (!val) {
            setError(`${f.label || f.name} is required.`);
            setLoading(false);
            return;
          }
          creds[f.name] = val;
        }
        body.credential = JSON.stringify(creds);
      } else if (shape === "api-key" && entry.requires_credential && !viaNode) {
        body.credential = credential;
      }
      // `no-auth` / `oauth` / `device-code` skip credential entirely;
      // OAuth / device-code placeholder creation lives in the sub-
      // flow components and doesn't reach this branch.
      if (entry.requires_gateway_url || endpointUrl) {
        body.endpoint_url = endpointUrl;
      }
      if (viaNode) {
        body.node_id = viaNode;
      }
      if (targetOrgId) {
        body.target_org_id = targetOrgId;
      }

      // Reserve the destructive action server-side before creating
      // the service. Without this, a refresh after `POST /keys`
      // succeeded but before `AiKeyAckPanel` posts `/complete` lets
      // the pairing's confirm form fire again, creating a duplicate
      // UserService / UserApiKey and orphaning the first one from
      // the CLI's ack. Throws 409/404 in stale-tab scenarios so the
      // surrounding try/catch aborts before the duplicate POST.
      await reservePairingAction(pairingId);
      // Rewind the reservation if `POST /keys` fails (e.g. backend
      // rejects a malformed label or credential); the user can fix
      // the input in place and retry without re-running the CLI.
      const res = await withRewindOnError(pairingId, () =>
        api.post<UserServiceCreated>("/keys", body),
      );
      onSuccess({
        kind: "ai-key",
        service_id: res.id,
        slug: res.slug,
        label: res.label,
      });
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Failed to create service.");
    } finally {
      setLoading(false);
    }
  }

  // Both "unsupported" fallbacks below navigate the user away from
  // this page; if we don't explicitly cancel the pairing, the
  // waiting CLI keeps polling until its TTL expires (up to 15
  // minutes) and the main Keys page has no knowledge of the
  // pairing to complete it. Cancel first so the CLI's next poll
  // sees `Cancelled` and exits cleanly with a "please finish on
  // the web UI" message.
  async function cancelThenNavigate(href: string | null) {
    try {
      await api.post(
        `/cli-pairings/${encodeURIComponent(pairingId)}/cancel`,
        {},
      );
    } catch {
      // Fire-and-forget: if cancel fails (expired, already
      // cancelled, network), we still navigate. Worst case the CLI
      // times out the same way it did before this fix.
    }
    if (href) {
      // Mode A (locally-served wizard) runs on `127.0.0.1:<port>`;
      // that axum server only serves `/wizard` + `/api/proxy/*`, so a
      // bare relative href like `/keys?...` 404s. In that context we
      // have no accurate way to build the frontend's absolute URL
      // (split-origin deployments), so tell the user to open their
      // NyxID dashboard manually and leave this tab alone — the CLI
      // has already been cancelled.
      if (window.__WIZARD_BOOTSTRAP__?.context === "local") {
        alert(
          "This auth shape isn't supported in the CLI wizard. " +
            "Open your NyxID dashboard and complete setup on the " +
            "Keys page (tab: External Services). " +
            "You can close this tab now.",
        );
        return;
      }
      window.location.assign(href);
    } else {
      window.history.back();
    }
  }

  // SSH services use a distinct `service add-ssh` command in the CLI
  // (certificate issuance, not credential binding). The local wizard
  // refuses SSH too — keep parity and tell the user the right command.
  if (entry.service_type === "ssh") {
    return (
      <div className="flex flex-col gap-3">
        <p className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-[12px]">
          {entry.name} is an SSH service. Use{" "}
          <code>nyxid service add-ssh</code> from your CLI instead
          (certificate-based auth, not a credential binding).
        </p>
        <Button
          variant="outline"
          onClick={() => void cancelThenNavigate(null)}
        >
          Go back
        </Button>
      </div>
    );
  }

  // Unknown auth shape or missing provider_config_id for OAuth /
  // device-code. Fall back to the regular Keys page where the user
  // can complete setup in a different UI.
  if (
    shape === "other" ||
    ((shape === "oauth" || shape === "device-code") && !entry.provider_config_id)
  ) {
    // `tab=services` puts the user on the right tab; `slug=...` is
    // consumed by `KeysPage` to auto-open `AddKeyDialog` with the
    // catalog entry pre-selected so the user doesn't have to hunt
    // for it in the grid.
    const href =
      `/keys?tab=services&slug=${encodeURIComponent(entry.slug)}`;
    return (
      <div className="flex flex-col gap-3">
        <p className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-[12px]">
          {entry.name} uses <code>{entry.auth_method}</code> auth,
          which isn't supported via remote pairing. Complete setup on
          the main Keys page. Your CLI will receive a cancel and
          print a "finish in browser" hint.
        </p>
        <Button
          variant="primary"
          onClick={() => void cancelThenNavigate(href)}
          className="justify-center gap-2"
        >
          Open Keys page
          <ExternalLink className="h-4 w-4" />
        </Button>
      </div>
    );
  }

  // OAuth / device-code: the confirm form collects label only, then
  // the flow handoff component drives placeholder creation, provider
  // auth, and polling.
  // Resolve the effective endpoint URL: the user's `--endpoint-url`
  // override (either via prefill from the CLI or the in-form input
  // for `requires_gateway_url` services) wins over the catalog
  // default. Both OAuth and device-code placeholder creation need
  // this or self-hosted services will bind to the wrong host.
  const effectiveEndpointUrl = endpointUrl.trim() || prefill.endpoint_url;

  if (authFlowActive && shape === "oauth" && entry.provider_config_id) {
    return (
      <OAuthFlow
        providerId={entry.provider_config_id}
        slug={entry.slug}
        label={label}
        nodeId={prefill.via_node}
        targetOrgId={targetOrgId}
        endpointUrl={effectiveEndpointUrl}
        pairingId={pairingId}
        credentialMode={entry.credential_mode}
        documentationUrl={entry.documentation_url}
        onSuccess={onSuccess}
        onCancel={() => {
          setAuthFlowActive(false);
        }}
      />
    );
  }
  if (authFlowActive && shape === "device-code" && entry.provider_config_id) {
    return (
      <DeviceCodeFlow
        providerId={entry.provider_config_id}
        slug={entry.slug}
        label={label}
        nodeId={prefill.via_node}
        targetOrgId={targetOrgId}
        endpointUrl={effectiveEndpointUrl}
        pairingId={pairingId}
        documentationUrl={entry.documentation_url}
        onSuccess={onSuccess}
        onCancel={() => {
          setAuthFlowActive(false);
        }}
      />
    );
  }

  const needsCredentialInput = shape === "api-key" && entry.requires_credential;
  const submitLabel = (() => {
    if (loading) return "Creating...";
    if (viaNode) return "Connect via node";
    if (shape === "oauth") return "Continue with provider sign-in";
    if (shape === "device-code") return "Get device code";
    if (shape === "no-auth") return "Connect";
    return "Create service";
  })();

  // Submit is disabled until all required inputs are filled. The
  // per-shape rules mirror the validation inside `submit()`.
  const tokenExchangeComplete = (() => {
    if (shape !== "token-exchange") return true;
    const fields = entry.token_exchange_credential_fields ?? [];
    return fields.every((f) => (tokenFields[f.name] ?? "").trim().length > 0);
  })();
  const submitDisabled =
    loading ||
    !label.trim() ||
    (needsCredentialInput && !viaNode && !credential.trim()) ||
    (entry.requires_gateway_url && !endpointUrl.trim()) ||
    (!viaNode && !tokenExchangeComplete);

  function handleSubmit() {
    if (shape === "oauth" || shape === "device-code") {
      // Sub-flow components own placeholder creation so they can
      // short-circuit when a prior session already authorized.
      setAuthFlowActive(true);
    } else {
      // api-key, token-exchange, no-auth — all go through POST /keys.
      void submit();
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start gap-3 rounded-lg border bg-muted/30 p-3">
        {entry.icon_url ? (
          <img
            src={entry.icon_url}
            alt=""
            className="h-8 w-8 rounded"
            loading="lazy"
          />
        ) : null}
        <div className="flex flex-col gap-0.5">
          <h3 className="font-medium">{entry.name}</h3>
          {entry.description ? (
            <p className="text-xs text-muted-foreground">{entry.description}</p>
          ) : null}
          <p className="text-xs text-muted-foreground">
            Auth: <code>{entry.auth_method}</code>
          </p>
        </div>
      </div>

      <div className="flex flex-col gap-3">
        <Field label="Label" htmlFor="pair-aikey-label">
          <Input
            id="pair-aikey-label"
            value={label}
            onChange={(e) => {
              setLabel(e.target.value);
            }}
            autoFocus
          />
        </Field>

        {entry.requires_gateway_url ? (
          <Field label="Instance URL" htmlFor="pair-aikey-url">
            <Input
              id="pair-aikey-url"
              value={endpointUrl}
              onChange={(e) => {
                setEndpointUrl(e.target.value);
              }}
              placeholder="https://your-instance.example.com"
            />
          </Field>
        ) : null}

        {needsCredentialInput && !viaNode ? (
          <Field label="API key" htmlFor="pair-aikey-credential">
            <Input
              id="pair-aikey-credential"
              type="password"
              autoComplete="off"
              value={credential}
              onChange={(e) => {
                setCredential(e.target.value);
              }}
              placeholder="sk-..."
            />
            {entry.api_key_url ? (
              <a
                href={entry.api_key_url}
                target="_blank"
                rel="noopener noreferrer"
                className="mt-1 inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:underline"
              >
                Get an API key
                <ExternalLink className="h-3 w-3" />
              </a>
            ) : null}
          </Field>
        ) : null}

        {shape === "token-exchange" && !viaNode
          ? (entry.token_exchange_credential_fields ?? []).map((f) => (
              <Field
                key={f.name}
                label={f.label || f.name}
                htmlFor={`pair-aikey-tx-${f.name}`}
              >
                <Input
                  id={`pair-aikey-tx-${f.name}`}
                  type={f.secret ? "password" : "text"}
                  autoComplete="off"
                  value={tokenFields[f.name] ?? ""}
                  onChange={(e) => {
                    const val = e.target.value;
                    setTokenFields((prev) => ({ ...prev, [f.name]: val }));
                  }}
                  placeholder={f.placeholder ?? ""}
                />
              </Field>
            ))
          : null}

        {viaNode ? (
          <div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
            <p className="text-xs font-medium text-foreground">Routed via node</p>
            <code className="font-mono text-[11px] text-muted-foreground">
              {viaNode}
            </code>
            <p className="text-[11px] text-muted-foreground mt-1">
              Credential will be configured on the node agent. NyxID
              never sees or stores it.
            </p>
          </div>
        ) : null}

        {shape === "no-auth" ? (
          <p className="text-xs text-muted-foreground">
            This service doesn't need a credential. Click Connect to
            add it to your services.
          </p>
        ) : null}
      </div>

      {error ? <ErrorLine message={error} /> : null}

      <Button variant="primary" onClick={handleSubmit} disabled={submitDisabled}>
        {submitLabel}
      </Button>
    </div>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  readonly label: string;
  readonly htmlFor: string;
  readonly children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
    </div>
  );
}

function ErrorLine({ message }: { readonly message: string }) {
  return (
    <p className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">
      {message}
    </p>
  );
}
