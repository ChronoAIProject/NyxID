// Per-kind wizard panels. Each panel owns the "confirm action" step
// (review prefill, maybe tweak a field, click "Do it"), then hands a
// DisplayOnce-ready secret back to the parent page via `onSecret`.
//
// The parent fires the `/cli-pairings/{id}/complete` POST with the
// ack payload when the user finishes DisplayOnce — panels don't
// touch that endpoint.

import { useState } from "react";
import { Info } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
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
  NodeRegisterPrefill,
  RotatePrefill,
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
