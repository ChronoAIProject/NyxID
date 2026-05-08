// Per-kind wizard panels. Each panel owns the "confirm action" step
// (review prefill, maybe tweak a field, click "Do it"), then hands a
// DisplayOnce-ready secret back to the parent page via `onSecret`.
//
// The parent fires the `/cli-pairings/{id}/complete` POST with the
// ack payload when the user finishes DisplayOnce — panels don't
// touch that endpoint.

import { useEffect, useRef, useState } from "react";
import { Info, Plus, Trash2 } from "lucide-react";
import QRCode from "qrcode";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { api } from "@/lib/api-client";
import {
  reservePairingAction,
  withRewindOnError,
} from "@/pages/cli-pair/reserve-action";
import type {
  ApiKeyCreatePrefill,
  DeveloperAppCreatePrefill,
  NodeRegisterPrefill,
  RotatePrefill,
  ServiceAccountCreatePrefill,
} from "@/pages/cli-pair/types";
import { NameInput } from "./name-input";
import { apiKeyNameSchema, nodeNameSchema, PLATFORMS } from "@/schemas/cli-wizard";
import { ScopePicker } from "./scope-picker";
import {
  AccessScopeCard,
  type AccessScopeState,
} from "./access-scope-card";
import { API_KEY_SCOPES, type ApiKeyScope } from "@/schemas/api-keys";
import { useOrgs } from "@/hooks/use-orgs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

// ── api-key-create panel ────────────────────────────────────────────

interface ApiKeyCreateResponse {
  readonly id: string;
  readonly full_key: string;
}

export interface ApiKeyCreateSuccess {
  readonly kind: "api-key-create";
  readonly api_key_id: string;
  readonly full_key: string;
}

interface ApiKeyCreateConfirmProps {
  readonly prefill: ApiKeyCreatePrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: ApiKeyCreateSuccess) => void;
}

/**
 * Parse the CLI-sent space-separated scope string into an initial
 * `Set<ApiKeyScope>`. Unknown entries are dropped so a malformed
 * prefill doesn't leave the chip row in an impossible state.
 */
function initialScopeSet(prefillScopes: string | undefined): Set<ApiKeyScope> {
  const fromPrefill = (prefillScopes ?? "read write")
    .split(/\s+/)
    .map((s) => s.trim())
    .filter(Boolean) as string[];
  const allowed = new Set<string>(API_KEY_SCOPES);
  return new Set(fromPrefill.filter((s) => allowed.has(s)) as ApiKeyScope[]);
}

function initialAccessScope(prefill: ApiKeyCreatePrefill): AccessScopeState {
  const servicesCsv = prefill.allowed_services_csv;
  const nodesCsv = prefill.allowed_nodes_csv;

  const selectedServiceIds = new Set(
    (servicesCsv ?? "")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
  );
  const selectedNodeIds = new Set(
    (nodesCsv ?? "")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
  );

  // Default to "Allow all" when the CLI didn't pass an explicit
  // allowed_*_csv. Presence of a CSV is what tells us the user wants
  // a scoped key (matches `nyxid api-key create --allowed-services
  // a,b,c` CLI semantics). The raw `prefill.allow_all_*` flag from
  // the CLI's ApiKeyCreatePrefill struct defaults to `false` in Rust,
  // so we can't trust it as "user explicitly unticked" — we have to
  // infer from the CSV presence instead.
  const allowAllServices = !servicesCsv;
  const allowAllNodes = !nodesCsv;

  return {
    allowAllServices,
    allowAllNodes,
    selectedServiceIds,
    selectedNodeIds,
  };
}

export function ApiKeyCreateConfirm({
  prefill,
  pairingId,
  onSuccess,
}: ApiKeyCreateConfirmProps) {
  const [name, setName] = useState(prefill.name ?? "");
  const [platform, setPlatform] = useState(prefill.platform ?? "");
  const [nameValid, setNameValid] = useState(false);
  const [scopes, setScopes] = useState<Set<ApiKeyScope>>(() =>
    initialScopeSet(prefill.scopes),
  );
  const [access, setAccess] = useState<AccessScopeState>(() =>
    initialAccessScope(prefill),
  );
  // Owner picker — an empty string means "personal account"; anything
  // else is an org id. Prefilled from `--org` if the CLI sent one.
  // Matches the wizard v3.2 behavior from the old vanilla JS wizard
  // (wizard.js:1938-2040) so org admins who run the command without
  // the flag can still switch ownership in the browser.
  const [ownerId, setOwnerId] = useState(prefill.org_id ?? "");
  const orgs = useOrgs();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      const body: Record<string, unknown> = {
        name,
        scopes: Array.from(scopes).join(" "),
        allow_all_services: access.allowAllServices,
        allow_all_nodes: access.allowAllNodes,
      };
      if (platform) body.platform = platform;
      if (prefill.callback_url) body.callback_url = prefill.callback_url;
      if (ownerId) body.target_org_id = ownerId;
      if (!access.allowAllServices) {
        body.allowed_service_ids = Array.from(access.selectedServiceIds);
      }
      if (!access.allowAllNodes) {
        body.allowed_node_ids = Array.from(access.selectedNodeIds);
      }
      // Match the terminal path's convention (`cli/src/commands/api_key.rs`):
      // a positive `expires_in_days` becomes an RFC-3339 `expires_at`; a
      // value of `0` means "no expiry" and is omitted so the backend
      // stores null (not an already-expired key).
      if (prefill.expires_in_days != null && prefill.expires_in_days > 0) {
        const expiry = new Date();
        expiry.setDate(expiry.getDate() + prefill.expires_in_days);
        body.expires_at = expiry.toISOString();
      }

      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<ApiKeyCreateResponse>("/api-keys", body),
      );
      onSuccess({
        kind: "api-key-create",
        api_key_id: res.id,
        full_key: res.full_key,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  const submitDisabled = loading || !nameValid || scopes.size === 0;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Create an API key
        </h2>
        <p className="text-sm text-muted-foreground">
          Review the details your CLI sent and confirm to mint the key.
        </p>
      </div>
      <div className="flex flex-col gap-4">
        <NameInput
          id="pair-api-key-name"
          label="Name"
          schema={apiKeyNameSchema}
          value={name}
          onChange={setName}
          onValidityChange={setNameValid}
          placeholder="e.g. coding-agent"
          hint="A short label so you can find this key in `nyxid api-key list`."
          autoFocus
        />
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-1.5">
            <Label htmlFor="pair-api-key-platform">Platform</Label>
            <TooltipProvider delayDuration={150}>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    aria-label="About platform tags"
                    className="text-muted-foreground transition-colors hover:text-foreground"
                  >
                    <Info className="h-3.5 w-3.5" />
                  </button>
                </TooltipTrigger>
                <TooltipContent
                  side="right"
                  align="start"
                  sideOffset={8}
                  className="max-w-[340px] whitespace-normal px-5 py-4 text-[13px] leading-[1.55]"
                >
                  <div className="flex flex-col gap-4">
                    <p>
                      <span className="font-medium text-foreground">
                        Platform
                      </span>{" "}
                      tags the key with the AI agent that will use it.
                    </p>
                    <p className="text-muted-foreground">
                      It controls three things: audit attribution (logs
                      show which agent made each proxy request), per-
                      agent rate-limit buckets, and dashboard filtering
                      on the API Keys page.
                    </p>
                    <p className="text-muted-foreground">
                      Values are a fixed allowlist —{" "}
                      <code>claude-code</code>, <code>cursor</code>,{" "}
                      <code>codex</code>, <code>openclaw</code>,{" "}
                      <code>generic</code>. Custom strings are rejected
                      by the backend.
                    </p>
                    <p className="text-muted-foreground">
                      Leave as <code>— none —</code> if you don't want
                      the tag.
                    </p>
                  </div>
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <Select
            value={platform === "" ? "__none__" : platform}
            onValueChange={(v) => {
              setPlatform(v === "__none__" ? "" : v);
            }}
          >
            <SelectTrigger id="pair-api-key-platform">
              <SelectValue placeholder="— none —" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">— none —</SelectItem>
              {PLATFORMS.map((p) => (
                <SelectItem key={p} value={p}>
                  {p}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Tags the key for audit attribution + per-agent rate limits.
          </p>
        </div>
        {(orgs.data?.length ?? 0) > 0 ? (
          <Field label="Owner" htmlFor="pair-api-key-owner">
            <select
              id="pair-api-key-owner"
              value={ownerId}
              onChange={(e) => {
                setOwnerId(e.target.value);
              }}
              className="flex h-10 w-full rounded-[10px] border border-input bg-transparent px-[14px] py-2 text-[13px] text-foreground ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            >
              <option value="">Personal (your account)</option>
              {orgs.data?.map((org) => (
                <option key={org.id} value={org.id}>
                  Org · {org.display_name ?? org.id}
                </option>
              ))}
            </select>
            <p className="mt-1 text-xs text-muted-foreground">
              Org-owned keys authenticate as the org; every admin of
              the selected org can rotate or delete them.
            </p>
          </Field>
        ) : null}
        <ScopePicker value={scopes} onChange={setScopes} />
        <AccessScopeCard value={access} onChange={setAccess} />
      </div>
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={submitDisabled}>
        {loading ? "Creating..." : "Create key"}
      </Button>
    </div>
  );
}

// ── api-key-rotate panel ────────────────────────────────────────────

interface ApiKeyRotateResponse {
  readonly id: string;
  readonly full_key: string;
}

export interface ApiKeyRotateSuccess {
  readonly kind: "api-key-rotate";
  readonly resource_id: string;
  readonly full_key: string;
}

interface ApiKeyRotateConfirmProps {
  readonly prefill: RotatePrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: ApiKeyRotateSuccess) => void;
}

export function ApiKeyRotateConfirm({
  prefill,
  pairingId,
  onSuccess,
}: ApiKeyRotateConfirmProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<ApiKeyRotateResponse>(
          `/api-keys/${encodeURIComponent(prefill.resource_id)}/rotate`,
        ),
      );
      onSuccess({
        kind: "api-key-rotate",
        resource_id: res.id,
        full_key: res.full_key,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Rotate API key
        </h2>
        <p className="text-sm text-muted-foreground">
          Rotating <strong>{prefill.display_name}</strong> will issue a
          new key and immediately revoke the previous one.
        </p>
      </div>
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={loading}>
        {loading ? "Rotating..." : "Rotate key"}
      </Button>
    </div>
  );
}

// ── node-register-token panel ───────────────────────────────────────

interface NodeRegisterTokenResponse {
  readonly token_id: string;
  readonly token: string;
  readonly name: string;
}

export interface NodeRegisterSuccess {
  readonly kind: "node-register-token";
  readonly token_id: string;
  readonly token: string;
}

interface NodeRegisterConfirmProps {
  readonly prefill: NodeRegisterPrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: NodeRegisterSuccess) => void;
}

/**
 * Must match the fallback used by the terminal path in
 * `cli/src/commands/node.rs::RegisterToken`. The backend requires a
 * non-empty lowercase name, so we can't send `undefined` — both
 * transports default to `my-node` when the user (or prefill) didn't
 * provide one, keeping the headless pairing flow usable for the
 * common no-flag invocation.
 */
const DEFAULT_NODE_NAME = "my-node";

export function NodeRegisterConfirm({
  prefill,
  pairingId,
  onSuccess,
}: NodeRegisterConfirmProps) {
  const [name, setName] = useState(prefill.name ?? "");
  const [nameValid, setNameValid] = useState(true); // empty is valid (we fall back to default)
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      const trimmed = name.trim();
      const effectiveName = trimmed.length > 0 ? trimmed : DEFAULT_NODE_NAME;
      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<NodeRegisterTokenResponse>(
          "/nodes/register-token",
          { name: effectiveName },
        ),
      );
      onSuccess({
        kind: "node-register-token",
        token_id: res.token_id,
        token: res.token,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Generate node registration token
        </h2>
        <p className="text-sm text-muted-foreground">
          Use this token with <code>nyxid node register</code> to
          connect a new node.
        </p>
      </div>
      <NameInput
        id="pair-node-name"
        label="Node name (optional)"
        schema={nodeNameSchema}
        value={name}
        onChange={setName}
        onValidityChange={setNameValid}
        placeholder={DEFAULT_NODE_NAME}
        hint={`Lowercase letters, digits, hyphens only (max 64). Leave blank for \`${DEFAULT_NODE_NAME}\`.`}
        optional
        autoFocus
      />
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={loading || !nameValid}>
        {loading ? "Generating..." : "Generate token"}
      </Button>
    </div>
  );
}

// ── node-rotate-token panel ─────────────────────────────────────────

interface NodeRotateTokenResponse {
  readonly auth_token: string;
  readonly signing_secret: string;
}

export interface NodeRotateSuccess {
  readonly kind: "node-rotate-token";
  readonly resource_id: string;
  readonly auth_token: string;
  readonly signing_secret: string;
}

interface NodeRotateConfirmProps {
  readonly prefill: RotatePrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: NodeRotateSuccess) => void;
}

export function NodeRotateConfirm({
  prefill,
  pairingId,
  onSuccess,
}: NodeRotateConfirmProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<NodeRotateTokenResponse>(
          `/nodes/${encodeURIComponent(prefill.resource_id)}/rotate-token`,
        ),
      );
      onSuccess({
        kind: "node-rotate-token",
        resource_id: prefill.resource_id,
        auth_token: res.auth_token,
        signing_secret: res.signing_secret,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Rotate node token
        </h2>
        <p className="text-sm text-muted-foreground">
          Rotating <strong>{prefill.display_name}</strong> issues a new
          auth token + signing secret and revokes the previous pair.
        </p>
      </div>
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={loading}>
        {loading ? "Rotating..." : "Rotate token"}
      </Button>
    </div>
  );
}

// ── service-account-create panel ────────────────────────────────────

interface ServiceAccountCreateResponse {
  readonly id: string;
  readonly client_id: string;
  readonly client_secret: string;
}

export interface ServiceAccountCreateSuccess {
  readonly kind: "service-account-create";
  readonly service_account_id: string;
  readonly client_id: string;
  readonly client_secret: string;
}

interface ServiceAccountCreateConfirmProps {
  readonly prefill: ServiceAccountCreatePrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: ServiceAccountCreateSuccess) => void;
}

export function ServiceAccountCreateConfirm({
  prefill,
  pairingId,
  onSuccess,
}: ServiceAccountCreateConfirmProps) {
  const [name, setName] = useState(prefill.name ?? "");
  const [scopes, setScopes] = useState(prefill.scopes ?? "openid profile");
  const [description, setDescription] = useState(prefill.description ?? "");
  const [roleIds, setRoleIds] = useState(prefill.role_ids_csv ?? "");
  const [ownerId, setOwnerId] = useState(prefill.org_id ?? "");
  const orgs = useOrgs();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      const body: Record<string, unknown> = {
        name: name.trim(),
        allowed_scopes: scopes.trim(),
      };
      if (description.trim()) body.description = description.trim();
      if (prefill.rate_limit_override != null) {
        body.rate_limit_override = prefill.rate_limit_override;
      }
      const roleList = roleIds
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      if (roleList.length > 0) body.role_ids = roleList;
      if (ownerId) body.target_org_id = ownerId;

      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<ServiceAccountCreateResponse>(
          "/admin/service-accounts",
          body,
        ),
      );
      onSuccess({
        kind: "service-account-create",
        service_account_id: res.id,
        client_id: res.client_id,
        client_secret: res.client_secret,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  const submitDisabled = loading || name.trim().length === 0 || scopes.trim().length === 0;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Create a service account
        </h2>
        <p className="text-sm text-muted-foreground">
          Service accounts authenticate via the OAuth client_credentials flow.
          The client_secret is shown once, on the next screen.
        </p>
      </div>
      <div className="flex flex-col gap-4">
        <Field label="Name" htmlFor="pair-sa-name">
          <Input
            id="pair-sa-name"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
            }}
            placeholder="e.g. ci-deploys"
            autoFocus
          />
        </Field>
        <Field label="Allowed scopes" htmlFor="pair-sa-scopes">
          <Input
            id="pair-sa-scopes"
            value={scopes}
            onChange={(e) => {
              setScopes(e.target.value);
            }}
            placeholder="openid profile"
          />
          <p className="text-xs text-muted-foreground">
            Space-separated. The SA may only request these scopes.
          </p>
        </Field>
        <Field label="Description (optional)" htmlFor="pair-sa-desc">
          <Input
            id="pair-sa-desc"
            value={description}
            onChange={(e) => {
              setDescription(e.target.value);
            }}
            placeholder="What this account is for"
          />
        </Field>
        <Field label="Role IDs (optional, comma-separated)" htmlFor="pair-sa-roles">
          <Input
            id="pair-sa-roles"
            value={roleIds}
            onChange={(e) => {
              setRoleIds(e.target.value);
            }}
            placeholder="role-id-1,role-id-2"
          />
        </Field>
        {(orgs.data?.length ?? 0) > 0 ? (
          <Field label="Owner" htmlFor="pair-sa-owner">
            <select
              id="pair-sa-owner"
              value={ownerId}
              onChange={(e) => {
                setOwnerId(e.target.value);
              }}
              className="flex h-10 w-full rounded-[10px] border border-input bg-transparent px-[14px] py-2 text-[13px] text-foreground ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            >
              <option value="">Personal (your admin account)</option>
              {orgs.data?.map((org) => (
                <option key={org.id} value={org.id}>
                  Org · {org.display_name ?? org.id}
                </option>
              ))}
            </select>
          </Field>
        ) : null}
      </div>
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={submitDisabled}>
        {loading ? "Creating..." : "Create service account"}
      </Button>
    </div>
  );
}

// ── service-account-rotate-secret panel ─────────────────────────────

interface ServiceAccountRotateSecretResponse {
  readonly client_id: string;
  readonly client_secret: string;
}

export interface ServiceAccountRotateSecretSuccess {
  readonly kind: "service-account-rotate-secret";
  readonly resource_id: string;
  readonly client_id: string;
  readonly client_secret: string;
}

interface ServiceAccountRotateSecretConfirmProps {
  readonly prefill: RotatePrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: ServiceAccountRotateSecretSuccess) => void;
}

export function ServiceAccountRotateSecretConfirm({
  prefill,
  pairingId,
  onSuccess,
}: ServiceAccountRotateSecretConfirmProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<ServiceAccountRotateSecretResponse>(
          `/admin/service-accounts/${encodeURIComponent(prefill.resource_id)}/rotate-secret`,
        ),
      );
      onSuccess({
        kind: "service-account-rotate-secret",
        resource_id: prefill.resource_id,
        client_id: res.client_id,
        client_secret: res.client_secret,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Rotate service account secret
        </h2>
        <p className="text-sm text-muted-foreground">
          Rotating <strong>{prefill.display_name}</strong> immediately revokes
          all existing access tokens issued under this service account and mints
          a new client_secret.
        </p>
      </div>
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={loading}>
        {loading ? "Rotating..." : "Rotate secret"}
      </Button>
    </div>
  );
}

// ── developer-app-create panel ──────────────────────────────────────

interface DeveloperAppCreateResponse {
  readonly id: string;
  readonly client_secret: string;
}

export interface DeveloperAppCreateSuccess {
  readonly kind: "developer-app-create";
  readonly developer_app_id: string;
  readonly client_secret: string;
}

interface DeveloperAppCreateConfirmProps {
  readonly prefill: DeveloperAppCreatePrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: DeveloperAppCreateSuccess) => void;
}

export function DeveloperAppCreateConfirm({
  prefill,
  pairingId,
  onSuccess,
}: DeveloperAppCreateConfirmProps) {
  const [name, setName] = useState(prefill.name ?? "");
  // Mutable list — each row gets a unique key so React doesn't
  // trample values when a row is removed mid-edit.
  const initialUris = (prefill.redirect_uris ?? []).filter(
    (u) => typeof u === "string" && u.length > 0,
  );
  const [redirectUris, setRedirectUris] = useState<string[]>(
    initialUris.length > 0 ? [...initialUris] : [""],
  );
  const [allowedScopes, setAllowedScopes] = useState(
    prefill.allowed_scopes ?? "openid profile email",
  );
  const [delegationScopes, setDelegationScopes] = useState(
    prefill.delegation_scopes ?? "",
  );
  const [brokerCapability, setBrokerCapability] = useState(
    prefill.broker_capability ?? false,
  );
  const [ownerId, setOwnerId] = useState(prefill.org_id ?? "");
  const orgs = useOrgs();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function updateUri(idx: number, value: string) {
    setRedirectUris((prev) => prev.map((u, i) => (i === idx ? value : u)));
  }
  function addUri() {
    setRedirectUris((prev) => [...prev, ""]);
  }
  function removeUri(idx: number) {
    setRedirectUris((prev) =>
      prev.length === 1 ? [""] : prev.filter((_, i) => i !== idx),
    );
  }

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      const cleanedUris = redirectUris.map((u) => u.trim()).filter(Boolean);
      if (cleanedUris.length === 0) {
        setError("At least one redirect URI is required.");
        setLoading(false);
        return;
      }

      const body: Record<string, unknown> = {
        name: name.trim(),
        redirect_uris: cleanedUris,
        // Wizard always mints confidential clients — public clients
        // bypass the wizard entirely (no client_secret to display).
        // The wizard proxy enforces this value at
        // `cli/src/wizard/server.rs::enforce_proxy_body_constraints`
        // so a tampered page can't downgrade to "public".
        client_type: "confidential",
      };
      if (allowedScopes.trim()) {
        // Backend's `CreateDeveloperOAuthClientRequest.allowed_scopes`
        // is `Option<Vec<String>>`, not a space-string.
        body.allowed_scopes = allowedScopes
          .split(/\s+/)
          .map((s) => s.trim())
          .filter(Boolean);
      }
      if (delegationScopes.trim()) {
        body.delegation_scopes = delegationScopes.trim();
      }
      if (brokerCapability) {
        body.broker_capability_enabled = true;
      }
      if (ownerId) body.target_org_id = ownerId;

      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<DeveloperAppCreateResponse>(
          "/developer/oauth-clients",
          body,
        ),
      );
      if (!res.client_secret) {
        // Defensive: the wizard is gated to confidential clients
        // upstream, so this branch only fires if the server returns
        // an unexpected response shape.
        throw new Error(
          "Server didn't return a client_secret — was the client_type 'public'?",
        );
      }
      onSuccess({
        kind: "developer-app-create",
        developer_app_id: res.id,
        client_secret: res.client_secret,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  const submitDisabled = loading || name.trim().length === 0;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Create a developer OAuth app
        </h2>
        <p className="text-sm text-muted-foreground">
          Confidential client — the client_secret is shown once on the next
          screen. Use it to sign Sign-in-with-NyxID requests from your
          downstream product.
        </p>
      </div>
      <div className="flex flex-col gap-4">
        <Field label="App name" htmlFor="pair-app-name">
          <Input
            id="pair-app-name"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
            }}
            placeholder="e.g. Acme Web"
            autoFocus
          />
        </Field>
        <div className="flex flex-col gap-1.5">
          <Label>Redirect URIs</Label>
          <div className="flex flex-col gap-2">
            {redirectUris.map((uri, idx) => (
              <div key={idx} className="flex items-center gap-2">
                <Input
                  value={uri}
                  onChange={(e) => {
                    updateUri(idx, e.target.value);
                  }}
                  placeholder="https://app.example.com/callback"
                  className="flex-1"
                />
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  onClick={() => {
                    removeUri(idx);
                  }}
                  aria-label="Remove redirect URI"
                  disabled={redirectUris.length === 1 && uri.trim().length === 0}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            ))}
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={addUri}
              className="self-start"
            >
              <Plus className="mr-1 h-3 w-3" /> Add redirect URI
            </Button>
          </div>
        </div>
        <Field label="Allowed scopes" htmlFor="pair-app-scopes">
          <Input
            id="pair-app-scopes"
            value={allowedScopes}
            onChange={(e) => {
              setAllowedScopes(e.target.value);
            }}
            placeholder="openid profile email"
          />
          <p className="text-xs text-muted-foreground">Space-separated.</p>
        </Field>
        <Field label="Delegation scopes (optional)" htmlFor="pair-app-delegation">
          <Input
            id="pair-app-delegation"
            value={delegationScopes}
            onChange={(e) => {
              setDelegationScopes(e.target.value);
            }}
            placeholder="(blank disables token exchange)"
          />
        </Field>
        <div className="flex items-center justify-between rounded-md border border-border bg-muted/20 px-3 py-2">
          <div className="flex flex-col gap-0.5">
            <Label htmlFor="pair-app-broker">Broker capability</Label>
            <p className="text-xs text-muted-foreground">
              Allow this app to broker downstream credentials.
            </p>
          </div>
          <Switch
            id="pair-app-broker"
            checked={brokerCapability}
            onCheckedChange={(v: boolean) => {
              setBrokerCapability(v);
            }}
          />
        </div>
        {(orgs.data?.length ?? 0) > 0 ? (
          <Field label="Owner" htmlFor="pair-app-owner">
            <select
              id="pair-app-owner"
              value={ownerId}
              onChange={(e) => {
                setOwnerId(e.target.value);
              }}
              className="flex h-10 w-full rounded-[10px] border border-input bg-transparent px-[14px] py-2 text-[13px] text-foreground ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            >
              <option value="">Personal</option>
              {orgs.data?.map((org) => (
                <option key={org.id} value={org.id}>
                  Org · {org.display_name ?? org.id}
                </option>
              ))}
            </select>
          </Field>
        ) : null}
      </div>
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={submitDisabled}>
        {loading ? "Creating..." : "Create app"}
      </Button>
    </div>
  );
}

// ── developer-app-rotate-secret panel ───────────────────────────────

interface DeveloperAppRotateSecretResponse {
  readonly id: string;
  readonly client_secret: string;
}

export interface DeveloperAppRotateSecretSuccess {
  readonly kind: "developer-app-rotate-secret";
  readonly resource_id: string;
  readonly client_secret: string;
}

interface DeveloperAppRotateSecretConfirmProps {
  readonly prefill: RotatePrefill;
  readonly pairingId: string;
  readonly onSuccess: (result: DeveloperAppRotateSecretSuccess) => void;
}

export function DeveloperAppRotateSecretConfirm({
  prefill,
  pairingId,
  onSuccess,
}: DeveloperAppRotateSecretConfirmProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setLoading(true);
    setError(null);
    try {
      await reservePairingAction(pairingId);
      const res = await withRewindOnError(pairingId, () =>
        api.post<DeveloperAppRotateSecretResponse>(
          `/developer/oauth-clients/${encodeURIComponent(prefill.resource_id)}/rotate-secret`,
        ),
      );
      onSuccess({
        kind: "developer-app-rotate-secret",
        resource_id: res.id,
        client_secret: res.client_secret,
      });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Rotate developer app secret
        </h2>
        <p className="text-sm text-muted-foreground">
          Rotating <strong>{prefill.display_name}</strong> mints a new
          client_secret. Update any deployments using the previous value
          immediately.
        </p>
      </div>
      {error ? <ErrorLine message={error} /> : null}
      <Button onClick={() => void submit()} disabled={loading}>
        {loading ? "Rotating..." : "Rotate secret"}
      </Button>
    </div>
  );
}

// ── mfa-setup panel (multi-step: setup → confirm → recovery codes) ──

interface MfaSetupResponse {
  readonly factor_id: string;
  readonly secret: string;
  readonly qr_code_url: string;
}

interface MfaConfirmResponse {
  readonly message: string;
  readonly recovery_codes: string[];
}

export interface MfaSetupSuccess {
  readonly kind: "mfa-setup";
  readonly factor_id: string;
  readonly recovery_codes: readonly string[];
}

interface MfaSetupConfirmProps {
  readonly pairingId: string;
  readonly onSuccess: (result: MfaSetupSuccess) => void;
}

/**
 * Multi-step enrollment confirm panel:
 *   1. on mount → POST /mfa/setup, render QR + secret
 *   2. user types TOTP code → POST /mfa/confirm
 *   3. on confirm success → call onSuccess with factor_id +
 *      recovery_codes (parent renders DisplayOnce variant)
 *
 * Both halves of the flow live in the browser; the CLI never sees
 * the TOTP secret, the QR URL, or the recovery codes. The
 * `MfaSetupPrefill` shape is empty today so the panel takes no
 * prefill prop — re-add it when CLI flags grow.
 */
export function MfaSetupConfirm({
  pairingId,
  onSuccess,
}: MfaSetupConfirmProps) {
  // Phase: "init" before reserve+setup, "ready" after secret is in
  // hand and we're waiting on the user's TOTP code, "confirming"
  // while /mfa/confirm is in flight.
  const [phase, setPhase] = useState<"init" | "ready" | "confirming">(
    "init",
  );
  const [factorId, setFactorId] = useState<string | null>(null);
  const [secret, setSecret] = useState<string | null>(null);
  const [qrUrl, setQrUrl] = useState<string | null>(null);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [secretRevealed, setSecretRevealed] = useState(false);
  // Guard against StrictMode double-invocation: the initial /mfa/setup
  // mints a fresh factor server-side, and we don't want two pending
  // factors per session. `started` flips on first attempt; cleanup or
  // a thrown error rewinds it so the user can retry.
  const startedRef = useRef(false);

  useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;
    void (async () => {
      try {
        await reservePairingAction(pairingId);
        const res = await withRewindOnError(pairingId, () =>
          api.post<MfaSetupResponse>("/auth/mfa/setup", {}),
        );
        setFactorId(res.factor_id);
        setSecret(res.secret);
        setQrUrl(res.qr_code_url);
        try {
          const dataUrl = await QRCode.toDataURL(res.qr_code_url, {
            width: 240,
            margin: 1,
          });
          setQrDataUrl(dataUrl);
        } catch {
          // QR rendering is a convenience; the otpauth URL is
          // displayed below as a fallback.
        }
        setPhase("ready");
      } catch (e) {
        setError(errorMessage(e));
        // Allow the user to retry once they fix the underlying issue.
        startedRef.current = false;
      }
    })();
  }, [pairingId]);

  async function submitConfirm() {
    if (!factorId || phase !== "ready") return;
    setError(null);
    setPhase("confirming");
    try {
      const trimmed = code.trim();
      if (trimmed.length === 0) {
        setError("Enter the 6-digit code from your authenticator.");
        setPhase("ready");
        return;
      }
      const res = await api.post<MfaConfirmResponse>("/auth/mfa/confirm", {
        code: trimmed,
      });
      onSuccess({
        kind: "mfa-setup",
        factor_id: factorId,
        recovery_codes: res.recovery_codes,
      });
    } catch (e) {
      setError(errorMessage(e));
      setPhase("ready");
    }
  }

  if (phase === "init") {
    return (
      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-1">
          <h2 className="font-serif text-[28px] font-normal">
            Setting up MFA
          </h2>
          <p className="text-sm text-muted-foreground">
            Generating a TOTP secret on the server…
          </p>
        </div>
        {error ? <ErrorLine message={error} /> : null}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 className="font-serif text-[28px] font-normal">
          Add MFA to your account
        </h2>
        <p className="text-sm text-muted-foreground">
          Scan this QR with your authenticator app (1Password, Authy, Google
          Authenticator). Then enter the 6-digit code it shows to verify and
          finish enrollment.
        </p>
      </div>

      {qrDataUrl ? (
        <div className="flex justify-center rounded-md border border-border bg-white p-4 dark:bg-muted/30">
          <img
            src={qrDataUrl}
            alt="MFA enrollment QR code"
            className="h-60 w-60"
          />
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          Couldn't render the QR code. Use the otpauth URL below instead.
        </p>
      )}

      <div className="flex flex-col gap-1.5">
        <Label>Or enter the secret manually</Label>
        <div className="flex items-center gap-2">
          <code className="flex-1 overflow-x-auto rounded-md border bg-muted/40 px-3 py-2 font-mono text-sm">
            {secretRevealed ? secret : "•".repeat(Math.max(secret?.length ?? 12, 12))}
          </code>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              setSecretRevealed((r) => !r);
            }}
          >
            {secretRevealed ? "Hide" : "Reveal"}
          </Button>
        </div>
        {qrUrl ? (
          <p className="break-all text-[11px] text-muted-foreground">
            otpauth URL: <code className="font-mono">{qrUrl}</code>
          </p>
        ) : null}
      </div>

      <Field label="6-digit code from your authenticator" htmlFor="pair-mfa-code">
        <Input
          id="pair-mfa-code"
          value={code}
          onChange={(e) => {
            setCode(e.target.value);
          }}
          placeholder="123456"
          inputMode="numeric"
          autoFocus
          maxLength={10}
          className="font-mono tracking-widest"
        />
      </Field>

      {error ? <ErrorLine message={error} /> : null}
      <Button
        onClick={() => void submitConfirm()}
        disabled={phase === "confirming" || code.trim().length === 0}
      >
        {phase === "confirming" ? "Verifying..." : "Verify and enable MFA"}
      </Button>
    </div>
  );
}

// ── helpers ─────────────────────────────────────────────────────────

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
    <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      {message}
    </p>
  );
}

function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  return "Something went wrong. Please try again.";
}
