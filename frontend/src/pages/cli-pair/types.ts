// Remote CLI pairing: request/response types for /api/v1/cli-pairings.
//
// These shapes are shared between the page's claim/complete/cancel
// mutations and the wizard panels that render per-kind confirmation UIs.

/**
 * Supported pairing kinds. These strings MUST match both the CLI's
 * `PairingFlow::kind()` (cli/src/wizard/pairing.rs) and the wizard
 * server's `FlowKind::slug` (cli/src/wizard/server.rs) so a single
 * pairing record can hop from CLI → server → frontend without a
 * string-mismatch bug.
 */
export type PairingKind =
  | "ai-key"
  | "api-key-create"
  | "api-key-rotate"
  | "node-register-token"
  | "node-rotate-token";

export function isPairingKind(v: unknown): v is PairingKind {
  return (
    v === "ai-key" ||
    v === "api-key-create" ||
    v === "api-key-rotate" ||
    v === "node-register-token" ||
    v === "node-rotate-token"
  );
}

/**
 * Shape of `POST /api/v1/cli-pairings/claim` response. `prefill` is
 * opaque (per-kind); each wizard panel narrows it with Zod at the
 * render site.
 *
 * `resumed: true` signals this is a re-claim of an already-claimed
 * record (refresh / second tab). Combined with `action_started`,
 * the frontend distinguishes:
 *
 *   - `resumed: true, action_started: false` — user refreshed in
 *     the pre-action window. Safe to re-render the confirm form;
 *     replaying the destructive API call is a no-op because it
 *     hasn't happened yet.
 *   - `resumed: true, action_started: true` — the mint/rotate
 *     already executed. The frontend MUST block re-entry: replaying
 *     would invalidate the secret the user already saved.
 *   - `resumed: false` — fresh claim. Normal path.
 */
export interface ClaimResponse {
  readonly id: string;
  readonly kind: PairingKind;
  readonly prefill: Record<string, unknown>;
  readonly resumed: boolean;
  readonly action_started: boolean;
}

/**
 * Ack payloads mirror `cli/src/wizard/mod.rs`. Each has a narrow
 * shape with `acknowledged: true`; the CLI's Rust decoder uses
 * `deny_unknown_fields` so extra keys are rejected. Keep these types
 * in sync with the Rust ones.
 */
export interface ApiKeyCreateAck {
  readonly acknowledged: true;
  readonly api_key_id: string;
}

export interface RotationAck {
  readonly acknowledged: true;
  readonly resource_id: string;
}

export interface NodeRegisterAck {
  readonly acknowledged: true;
  readonly token_id: string;
}

/**
 * Ack for the ai-key (service-add) pairing flow. Unlike the
 * DisplayOnce acks this carries a handful of non-secret identifiers
 * the CLI prints in its "service created" summary. The downstream
 * credential (API key pasted by the user) is deliberately NOT part
 * of the ack — it's stored server-side as part of the `UserApiKey`
 * record and the CLI never sees it.
 *
 * Intentionally no `proxy_url`: the frontend runs on a different
 * origin than the backend on split-origin deployments, so the CLI
 * builds the proxy URL itself from its authoritative base_url.
 */
export interface AiKeyAck {
  readonly acknowledged: true;
  readonly service_id: string;
  readonly slug: string;
  readonly label: string;
}

export type AckPayload =
  | ApiKeyCreateAck
  | RotationAck
  | NodeRegisterAck
  | AiKeyAck;

/**
 * Per-kind prefill shapes — only the fields the CLI sends today.
 * Missing fields are fine; the wizard falls back to blank inputs.
 */
export interface ApiKeyCreatePrefill {
  readonly name?: string;
  readonly platform?: string;
  readonly scopes?: string;
  readonly expires_in_days?: number;
  readonly allow_all_services?: boolean;
  readonly allow_all_nodes?: boolean;
  readonly allowed_services_csv?: string;
  readonly allowed_nodes_csv?: string;
  readonly callback_url?: string;
  readonly org_id?: string;
}

export interface RotatePrefill {
  readonly resource_id: string;
  readonly display_name: string;
}

export interface NodeRegisterPrefill {
  readonly name?: string;
}

/**
 * Prefill sent by the CLI for `nyxid service add`. Mirrors the URL
 * query-string the local-server wizard uses, and the `prefill_ai_key`
 * JSON shape from `cli/src/wizard/pairing.rs`.
 *
 * Issue #414 — `--custom` mode adds four optional fields the wizard
 * SPA reads to skip the catalog grid entirely and render the custom-
 * service form directly with the user's CLI-supplied definitional
 * values pre-populated.
 */
export interface AiKeyPrefill {
  readonly slug?: string;
  readonly label?: string;
  readonly via_node?: string;
  readonly endpoint_url?: string;
  /** `true` when `--custom` was passed at the CLI. The SPA skips
   *  Step 1 (catalog grid) and renders `CustomServiceForm` on first
   *  render. */
  readonly custom?: boolean;
  /** `--slug <s>` override for custom services. Distinct from
   *  `slug` above which selects a catalog entry. */
  readonly custom_slug?: string;
  /** Auth method (bearer, header, query, path, basic, body,
   *  bot_bearer, none). Required for custom services since there's
   *  no catalog default to inherit. */
  readonly auth_method?: string;
  /** Auth key name (e.g. Authorization, X-API-Key, app_secret).
   *  Defaults are derived per auth_method by the SPA when unset. */
  readonly auth_key_name?: string;
}
