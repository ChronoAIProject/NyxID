import { buildRciContext, encrypt } from "@/lib/crypto";
import {
  credentialAcceptFingerprintSha384Hex,
  isValidSha384Hex,
  pathFromSameOriginScriptUrl,
  type CredentialAcceptScriptBytes,
} from "@/lib/release-integrity/manifest";
import {
  runtimeConfigSchema,
  type RuntimeConfig,
} from "@/schemas/runtime-config";
import type { CiphertextEnvelope } from "@/lib/crypto";
import type {
  FanOutPendingCredentialCiphertextResponse,
  FanOutPendingCredentialPubkeysResponse,
  FanOutPendingCredentialResponse,
  NodePendingCredentialCiphertextResponse,
  NodePendingCredentialInfo,
  NodePendingCredentialPubkeyResponse,
  NodePendingCredentialRemoteState,
} from "@/types/nodes";

const PUBKEY_WAIT_MS = 30_000;
const POLL_WAIT_MS = 60_000;
const PUBKEY_DELAYS_MS = [500, 1_000, 2_000, 4_000, 8_000] as const;
const POLL_DELAYS_MS = [1_000, 2_000, 3_000, 5_000] as const;
const MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE = 16 * 1024 - 16;
const VERIFICATION_SESSION_PREFIX = "nyxid:rci-accept:verification:v1:";

type AcceptStatus =
  | "idle"
  | "waiting_pubkey"
  | "encrypting"
  | "posting"
  | "polling"
  | "consumed"
  | "partial_decrypted"
  | "decrypt_failed"
  | "expired"
  | "declined"
  | "timeout"
  | "legacy_fallback"
  | "error";

interface PendingCredentialListResponse {
  readonly pending_credentials: readonly PendingCredentialWithState[];
}

type PendingCredentialWithState = NodePendingCredentialInfo & {
  readonly remote_state?: NodePendingCredentialRemoteState | null;
};

type IntegrityVerification =
  | {
      readonly mode: "admin_verified";
      readonly fingerprint_sha384_hex: string;
      readonly verified_at: string;
      readonly manifest_url_configured: true;
    }
  | {
      readonly mode: "org_policy_opt_out";
      readonly fingerprint_sha384_hex: null;
      readonly verified_at: null;
      readonly manifest_url_configured: boolean;
    };

interface VerificationSession {
  readonly fingerprint_sha384_hex: string;
  readonly verified_at: string;
  readonly manifest_url: string | null;
}

interface RouteInfo {
  readonly nodeId: string | null;
  readonly pendingId: string;
  readonly fanOut: boolean;
}

interface CredentialAcceptDeps {
  readonly fetch: typeof fetch;
  readonly location: Location;
  readonly window: Window;
  readonly document: Document;
  readonly storage: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  readonly now: () => number;
  readonly delay: (ms: number) => Promise<void>;
}

interface IntegrityState {
  readonly runtimeConfig: RuntimeConfig;
  readonly fingerprintSha384Hex: string;
  readonly shortFingerprint: string;
  readonly optOut: boolean;
  readonly verifiedAt: string | null;
}

interface PageState {
  route: RouteInfo;
  integrity: IntegrityState | null;
  status: AcceptStatus;
  errorMessage: string | null;
  formVisible: boolean;
  busy: boolean;
  fanOutResponse:
    | FanOutPendingCredentialCiphertextResponse
    | FanOutPendingCredentialResponse
    | null;
  ciphertextResponse: NodePendingCredentialCiphertextResponse | null;
  retryPlaintext: Uint8Array | null;
}

interface FanOutCiphertextFlowInput {
  readonly route: RouteInfo;
  readonly pending: () => Promise<FanOutPendingCredentialResponse> | FanOutPendingCredentialResponse;
  readonly targetNodeIds: ReadonlySet<string> | null;
  readonly integrityVerification: IntegrityVerification;
}

class ApiError extends Error {
  readonly status: number;
  readonly errorCode: number | null;

  constructor(status: number, message: string, errorCode: number | null) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.errorCode = errorCode;
  }
}

const statusLabel: Readonly<Record<AcceptStatus, string>> = {
  idle: "Ready",
  waiting_pubkey: "Waiting for node key",
  encrypting: "Encrypting",
  posting: "Sending ciphertext",
  polling: "Waiting for node",
  consumed: "Stored",
  partial_decrypted: "Partially stored",
  decrypt_failed: "Decrypt failed",
  expired: "Expired",
  declined: "Declined",
  timeout: "Timed out",
  legacy_fallback: "Manual setup",
  error: "Error",
};

const terminalDescriptions: Partial<Readonly<Record<AcceptStatus, string>>> = {
  consumed: "The node consumed the encrypted credential.",
  partial_decrypted: "Some nodes stored the credential and some still need retry.",
  decrypt_failed: "The node could not decrypt the submitted envelope.",
  expired: "This pending credential expired before completion.",
  declined: "The node operator declined this pending credential.",
  timeout: "The node did not report completion before the browser stopped waiting.",
  legacy_fallback:
    "Use the node CLI on the machine that runs the agent to enter the credential locally.",
};

function defaultDelay(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function nextDelay(delays: readonly number[], attempt: number): number {
  return delays[Math.min(attempt, delays.length - 1)] ?? delays[0] ?? 1_000;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function statusClass(status: AcceptStatus): string {
  if (status === "consumed") return "status success";
  if (status === "legacy_fallback" || status === "timeout") return "status warning";
  if (
    status === "partial_decrypted" ||
    status === "decrypt_failed" ||
    status === "expired" ||
    status === "declined" ||
    status === "error"
  ) {
    return "status danger";
  }
  return "status";
}

function isPubkeyAwaiting(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 404 || error.errorCode === 8009);
}

function statusFromRemoteState(
  state: NodePendingCredentialRemoteState,
): AcceptStatus | null {
  if (state === "consumed") return "consumed";
  if (state === "partial_decrypted") return "partial_decrypted";
  if (state === "decrypt_failed") return "decrypt_failed";
  if (state === "expired") return "expired";
  if (state === "declined") return "declined";
  return null;
}

function clearRetryPlaintext(state: PageState): void {
  state.retryPlaintext?.fill(0);
  state.retryPlaintext = null;
}

function retainRetryPlaintext(state: PageState, plaintext: Uint8Array): void {
  clearRetryPlaintext(state);
  state.retryPlaintext = plaintext.slice();
}

function retainFanOutPlaintextForRetry(state: PageState, plaintext: Uint8Array): void {
  if (state.retryPlaintext !== plaintext) {
    retainRetryPlaintext(state, plaintext);
  }
}

function failedFanOutTargetIds(
  response: FanOutPendingCredentialCiphertextResponse | FanOutPendingCredentialResponse,
): Set<string> {
  return new Set(
    response.targets
      .filter((target) => target.remote_state === "decrypt_failed")
      .map((target) => target.node_id),
  );
}

function resolveTerminalState(
  pending: PendingCredentialWithState,
  now: number,
): AcceptStatus | null {
  if (pending.consumed_at || pending.remote_state === "consumed") return "consumed";
  if (pending.declined_at) return "declined";
  if (pending.remote_state === "decrypt_failed") return "decrypt_failed";
  if (pending.remote_state === "expired") return "expired";
  if (Date.parse(pending.expires_at) <= now) return "expired";
  if (!pending.is_active && !pending.consumed_at && !pending.declined_at) return "expired";
  return null;
}

export function parseCredentialAcceptRoute(pathname: string): RouteInfo {
  const direct = pathname.match(
    /^\/nodes\/([^/]+)\/credentials\/pending\/([^/]+)\/accept$/,
  );
  if (direct?.[1] && direct[2]) {
    return {
      nodeId: decodeURIComponent(direct[1]),
      pendingId: decodeURIComponent(direct[2]),
      fanOut: false,
    };
  }
  const fanOut = pathname.match(/^\/nodes\/credentials\/pending\/([^/]+)\/fan-out\/accept$/);
  if (fanOut?.[1]) {
    return {
      nodeId: null,
      pendingId: decodeURIComponent(fanOut[1]),
      fanOut: true,
    };
  }
  throw new Error("Unsupported credential accept URL.");
}

function safeReturnTo(value: string | null, origin: string): string | null {
  if (!value || value.length > 2048) return null;
  if (!value.startsWith("/") || value.startsWith("//") || value.startsWith("/\\")) {
    return null;
  }
  try {
    const url = new URL(value, origin);
    if (url.origin !== origin) return null;
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    return null;
  }
}

async function apiFetch<T>(
  deps: CredentialAcceptDeps,
  endpoint: string,
  options: RequestInit = {},
): Promise<T> {
  const response = await deps.fetch(`/api/v1${endpoint}`, {
    ...options,
    credentials: "include",
    headers: {
      "Content-Type": "application/json",
      ...(options.headers ?? {}),
    },
  });
  if (!response.ok) {
    let message = `Request failed with status ${String(response.status)}`;
    let errorCode: number | null = null;
    try {
      const body = (await response.json()) as { message?: string; error_code?: number };
      message = body.message ?? message;
      errorCode = typeof body.error_code === "number" ? body.error_code : null;
    } catch {
      // Keep the generic message.
    }
    throw new ApiError(response.status, message, errorCode);
  }
  return (await response.json()) as T;
}

async function postJson<T>(
  deps: CredentialAcceptDeps,
  endpoint: string,
  body: unknown,
): Promise<T> {
  return apiFetch<T>(deps, endpoint, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

function verificationSessionKey(fingerprintSha384Hex: string): string {
  return `${VERIFICATION_SESSION_PREFIX}${fingerprintSha384Hex}`;
}

function readVerificationSession(
  deps: CredentialAcceptDeps,
  fingerprintSha384Hex: string,
  runtimeConfig: RuntimeConfig,
): VerificationSession | null {
  const raw = deps.storage.getItem(verificationSessionKey(fingerprintSha384Hex));
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as VerificationSession;
    if (parsed.fingerprint_sha384_hex !== fingerprintSha384Hex) return null;
    if (parsed.manifest_url !== runtimeConfig.release_integrity.manifest_url) return null;
    const verifiedAtMs = Date.parse(parsed.verified_at);
    if (!Number.isFinite(verifiedAtMs)) return null;
    const ttlMs = runtimeConfig.release_integrity.verification_ttl_secs * 1000;
    if (deps.now() - verifiedAtMs > ttlMs) return null;
    return parsed;
  } catch {
    deps.storage.removeItem(verificationSessionKey(fingerprintSha384Hex));
    return null;
  }
}

function writeVerificationSession(
  deps: CredentialAcceptDeps,
  fingerprintSha384Hex: string,
  runtimeConfig: RuntimeConfig,
): string {
  const verifiedAt = new Date(deps.now()).toISOString();
  const session: VerificationSession = {
    fingerprint_sha384_hex: fingerprintSha384Hex,
    verified_at: verifiedAt,
    manifest_url: runtimeConfig.release_integrity.manifest_url,
  };
  deps.storage.setItem(
    verificationSessionKey(fingerprintSha384Hex),
    JSON.stringify(session),
  );
  return verifiedAt;
}

async function computeLoadedScriptFingerprint(
  deps: CredentialAcceptDeps,
): Promise<string> {
  const scriptElements = Array.from(
    deps.document.querySelectorAll<HTMLScriptElement>(
      'script[data-nyx-integrity-role="credential_accept_script"][src]',
    ),
  );
  if (scriptElements.length === 0) {
    throw new Error("Credential accept script metadata was not found.");
  }

  const scripts: CredentialAcceptScriptBytes[] = [];
  for (const script of scriptElements) {
    const path = pathFromSameOriginScriptUrl(script.src, deps.location.href);
    const response = await deps.fetch(script.src, { credentials: "include" });
    if (!response.ok) {
      throw new Error("Failed to read loaded credential accept script bytes.");
    }
    scripts.push({
      path,
      bytes: new Uint8Array(await response.arrayBuffer()),
    });
  }
  return credentialAcceptFingerprintSha384Hex(scripts);
}

function createIntegrityVerification(state: PageState, deps: CredentialAcceptDeps): IntegrityVerification {
  const integrity = state.integrity;
  if (!integrity) {
    throw new Error("Integrity verification is not ready.");
  }
  if (integrity.optOut) {
    return {
      mode: "org_policy_opt_out",
      fingerprint_sha384_hex: null,
      verified_at: null,
      manifest_url_configured: integrity.runtimeConfig.release_integrity.enabled,
    };
  }
  if (!integrity.runtimeConfig.release_integrity.enabled) {
    throw new Error("Release integrity manifest URL is not configured.");
  }
  if (!integrity.verifiedAt || !isValidSha384Hex(integrity.fingerprintSha384Hex)) {
    throw new Error("Verify the release fingerprint before submitting.");
  }
  const ageMs = deps.now() - Date.parse(integrity.verifiedAt);
  if (ageMs > integrity.runtimeConfig.release_integrity.verification_ttl_secs * 1000) {
    throw new Error("Fingerprint verification expired. Verify it again.");
  }
  return {
    mode: "admin_verified",
    fingerprint_sha384_hex: integrity.fingerprintSha384Hex,
    verified_at: integrity.verifiedAt,
    manifest_url_configured: true,
  };
}

async function fetchRuntimeConfig(deps: CredentialAcceptDeps): Promise<RuntimeConfig> {
  return runtimeConfigSchema.parse(await apiFetch<unknown>(deps, "/runtime-config"));
}

async function fetchPubkeyWithBackoff(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
): Promise<NodePendingCredentialPubkeyResponse | null> {
  if (!route.nodeId) return null;
  const startedAt = deps.now();
  let attempt = 0;
  while (deps.now() - startedAt < PUBKEY_WAIT_MS) {
    try {
      return await apiFetch<NodePendingCredentialPubkeyResponse>(
        deps,
        `/nodes/${encodeURIComponent(route.nodeId)}/credentials/pending/${encodeURIComponent(route.pendingId)}`,
      );
    } catch (err) {
      if (!isPubkeyAwaiting(err)) throw err;
      const remaining = PUBKEY_WAIT_MS - (deps.now() - startedAt);
      if (remaining <= 0) break;
      await deps.delay(Math.min(nextDelay(PUBKEY_DELAYS_MS, attempt), remaining));
      attempt += 1;
    }
  }
  return null;
}

async function fetchPendingMetadata(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
): Promise<PendingCredentialWithState> {
  if (!route.nodeId) throw new Error("Node id is required.");
  const res = await apiFetch<PendingCredentialListResponse>(
    deps,
    `/nodes/${encodeURIComponent(route.nodeId)}/credentials/pending?include_history=true`,
  );
  const pending = res.pending_credentials.find((credential) => credential.id === route.pendingId);
  if (!pending) throw new Error("Pending credential metadata was not found.");
  return pending;
}

async function pollTerminalState(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
): Promise<AcceptStatus> {
  const startedAt = deps.now();
  let attempt = 0;
  while (deps.now() - startedAt < POLL_WAIT_MS) {
    const pending = await fetchPendingMetadata(deps, route);
    const terminal = resolveTerminalState(pending, deps.now());
    if (terminal) return terminal;
    const remaining = POLL_WAIT_MS - (deps.now() - startedAt);
    if (remaining <= 0) break;
    await deps.delay(Math.min(nextDelay(POLL_DELAYS_MS, attempt), remaining));
    attempt += 1;
  }
  return "timeout";
}

async function fetchFanOutStatus(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
): Promise<FanOutPendingCredentialResponse> {
  return apiFetch<FanOutPendingCredentialResponse>(
    deps,
    `/nodes/credentials/pending/${encodeURIComponent(route.pendingId)}/fan-out`,
  );
}

async function fetchFanOutPubkeysWithBackoff(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
): Promise<FanOutPendingCredentialPubkeysResponse | null> {
  const startedAt = deps.now();
  let attempt = 0;
  while (deps.now() - startedAt < PUBKEY_WAIT_MS) {
    const pubkeys = await apiFetch<FanOutPendingCredentialPubkeysResponse>(
      deps,
      `/nodes/credentials/pending/${encodeURIComponent(route.pendingId)}/fan-out/pubkeys`,
    );
    if (
      pubkeys.targets.length > 0 &&
      pubkeys.targets.every((target) => Boolean(target.node_pubkey))
    ) {
      return pubkeys;
    }
    const remaining = PUBKEY_WAIT_MS - (deps.now() - startedAt);
    if (remaining <= 0) break;
    await deps.delay(Math.min(nextDelay(PUBKEY_DELAYS_MS, attempt), remaining));
    attempt += 1;
  }
  return null;
}

async function fetchFanOutPubkeysForTargetsWithBackoff(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
  targetNodeIds: ReadonlySet<string>,
): Promise<FanOutPendingCredentialPubkeysResponse | null> {
  const startedAt = deps.now();
  let attempt = 0;
  while (deps.now() - startedAt < PUBKEY_WAIT_MS) {
    const pubkeys = await apiFetch<FanOutPendingCredentialPubkeysResponse>(
      deps,
      `/nodes/credentials/pending/${encodeURIComponent(route.pendingId)}/fan-out/pubkeys`,
    );
    const retryTargets = pubkeys.targets.filter((target) =>
      targetNodeIds.has(target.node_id),
    );
    if (
      retryTargets.length === targetNodeIds.size &&
      retryTargets.every(
        (target) =>
          Boolean(target.node_pubkey) && target.remote_state === "pubkey_posted",
      )
    ) {
      return pubkeys;
    }
    const remaining = PUBKEY_WAIT_MS - (deps.now() - startedAt);
    if (remaining <= 0) break;
    await deps.delay(Math.min(nextDelay(PUBKEY_DELAYS_MS, attempt), remaining));
    attempt += 1;
  }
  return null;
}

function encryptedFanOutItems(
  plaintext: Uint8Array,
  pending: Pick<
    FanOutPendingCredentialResponse,
    "service_slug" | "injection_method" | "field_name" | "target_url"
  >,
  pubkeys: FanOutPendingCredentialPubkeysResponse,
  targetNodeIds: ReadonlySet<string> | null,
) {
  const targets = targetNodeIds
    ? pubkeys.targets.filter((target) => targetNodeIds.has(target.node_id))
    : pubkeys.targets;
  if (targetNodeIds && targets.length !== targetNodeIds.size) {
    throw new Error("Fan-out retry target pubkeys are not ready.");
  }
  return targets.map((target) => {
    if (!target.node_pubkey) {
      throw new Error("Fan-out target pubkey is not ready.");
    }
    const context = buildRciContext({
      node_id: target.node_id,
      pending_credential_id: pubkeys.fanout_id,
      service_slug: pending.service_slug,
      injection_method: pending.injection_method,
      field_name: pending.field_name,
      target_url: pending.target_url ?? null,
      version: target.version,
    });
    return {
      node_id: target.node_id,
      generation: target.generation,
      ...encrypt(plaintext, target.node_pubkey, context),
    };
  });
}

async function pollFanOutTerminalState(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
): Promise<FanOutPendingCredentialResponse | null> {
  const startedAt = deps.now();
  let attempt = 0;
  while (deps.now() - startedAt < POLL_WAIT_MS) {
    const pending = await fetchFanOutStatus(deps, route);
    const terminal = statusFromRemoteState(pending.remote_state ?? "ciphertext_received");
    if (terminal) return pending;
    const remaining = POLL_WAIT_MS - (deps.now() - startedAt);
    if (remaining <= 0) break;
    await deps.delay(Math.min(nextDelay(POLL_DELAYS_MS, attempt), remaining));
    attempt += 1;
  }
  return null;
}

async function waitForFanOutPubkeys(
  deps: CredentialAcceptDeps,
  route: RouteInfo,
  targetNodeIds: ReadonlySet<string> | null,
): Promise<FanOutPendingCredentialPubkeysResponse | null> {
  return targetNodeIds
    ? fetchFanOutPubkeysForTargetsWithBackoff(deps, route, targetNodeIds)
    : fetchFanOutPubkeysWithBackoff(deps, route);
}

async function runFanOutCiphertextFlow(
  root: HTMLElement,
  state: PageState,
  deps: CredentialAcceptDeps,
  plaintext: Uint8Array,
  input: FanOutCiphertextFlowInput,
): Promise<void> {
  state.status = "waiting_pubkey";
  render(root, state, deps);
  const [pending, pubkeys] = await Promise.all([
    input.pending(),
    waitForFanOutPubkeys(deps, input.route, input.targetNodeIds),
  ]);
  if (!pubkeys) {
    state.status = "legacy_fallback";
    return;
  }

  state.status = "encrypting";
  render(root, state, deps);
  const items = encryptedFanOutItems(
    plaintext,
    pending,
    pubkeys,
    input.targetNodeIds,
  );

  state.status = "posting";
  render(root, state, deps);
  const postResult = await postJson<FanOutPendingCredentialCiphertextResponse>(
    deps,
    `/nodes/credentials/pending/${encodeURIComponent(input.route.pendingId)}/fan-out/ciphertexts`,
    {
      fan_out_revision: pubkeys.fan_out_revision,
      integrity_verification: input.integrityVerification,
      items,
    },
  );
  state.fanOutResponse = postResult;
  const directTerminal = statusFromRemoteState(postResult.remote_state);
  if (directTerminal) {
    state.status = directTerminal;
    if (directTerminal === "partial_decrypted") {
      retainFanOutPlaintextForRetry(state, plaintext);
    }
    return;
  }

  state.status = "polling";
  render(root, state, deps);
  const terminalPending = await pollFanOutTerminalState(deps, input.route);
  if (!terminalPending) {
    state.status = "timeout";
    return;
  }
  state.fanOutResponse = terminalPending;
  state.status =
    statusFromRemoteState(terminalPending.remote_state ?? "ciphertext_received") ?? "timeout";
  if (state.status === "partial_decrypted") {
    retainFanOutPlaintextForRetry(state, plaintext);
  }
}

function render(root: HTMLElement, state: PageState, deps: CredentialAcceptDeps): void {
  const terminalText = terminalDescriptions[state.status];
  const integrity = state.integrity;
  const manifestUrl = integrity?.runtimeConfig.release_integrity.manifest_url;
  const verifyDisabled = !manifestUrl || integrity?.optOut || state.busy;
  const canSubmit =
    state.formVisible &&
    !state.busy &&
    Boolean(integrity) &&
    (Boolean(integrity?.optOut) ||
      (Boolean(integrity?.runtimeConfig.release_integrity.enabled) &&
        Boolean(integrity?.verifiedAt)));
  const returnTo = safeReturnTo(
    new URLSearchParams(deps.location.search).get("return_to"),
    deps.location.origin,
  );
  const backTarget = returnTo ?? (state.route.nodeId ? `/nodes/${state.route.nodeId}` : "/nodes");

  root.innerHTML = `
    <section class="panel stack">
      <div class="row">
        <span class="${statusClass(state.status)}">${escapeHtml(statusLabel[state.status])}</span>
        ${
          state.ciphertextResponse
            ? `<span class="status">${escapeHtml(state.ciphertextResponse.delivery_status)}</span>`
            : ""
        }
        ${
          state.fanOutResponse
            ? `<span class="status">${String(state.fanOutResponse.targets.length)} targets</span>`
            : ""
        }
      </div>
      <div>
        <h1>Accept Credential</h1>
        <p>
          This standalone page detects credential-accept JavaScript substitution
          when an admin verifies the fingerprint against an independently signed
          release manifest. It does not prevent server-side HTML substitution
          and does not defend against node public-key substitution.
        </p>
      </div>
      ${
        integrity
          ? `<div class="panel stack">
              <div>
                <div class="muted">Release fingerprint</div>
                <code data-testid="fingerprint">${escapeHtml(integrity.shortFingerprint)}</code>
              </div>
              <div class="row">
                <button type="button" id="verify-link" ${verifyDisabled ? "disabled" : ""}>Open manifest</button>
                <span class="muted">${
                  integrity.optOut
                    ? "Org policy has opted out of per-session fingerprint verification."
                    : manifestUrl
                      ? "Compare this fingerprint with the signed manifest before accepting."
                      : "Release integrity manifest URL is not configured; submit is blocked unless org policy opts out."
                }</span>
              </div>
              <label class="check-row">
                <input type="checkbox" id="verified-checkbox" ${
                  integrity.optOut || integrity.verifiedAt ? "checked" : ""
                } ${integrity.optOut || !manifestUrl || state.busy ? "disabled" : ""} />
                <span>I verified the fingerprint out-of-band.</span>
              </label>
            </div>`
          : ""
      }
      ${state.errorMessage ? `<p class="status danger">${escapeHtml(state.errorMessage)}</p>` : ""}
      ${terminalText ? `<p class="muted">${escapeHtml(terminalText)}</p>` : ""}
      ${
        state.status === "legacy_fallback"
          ? `<div class="panel stack">
              <div class="muted">Node CLI</div>
              <code>nyxid node credentials pending</code>
              <code>nyxid node credentials accept &lt;service-slug&gt;</code>
            </div>`
          : ""
      }
      ${
        state.formVisible && state.status !== "legacy_fallback"
          ? `<form id="accept-form" class="stack">
              <div>
                <label for="credential-secret">Credential value</label>
                <input id="credential-secret" type="password" autocomplete="new-password" ${
                  state.busy ? "disabled" : ""
                } />
              </div>
              <div class="row">
                <button type="submit" ${canSubmit ? "" : "disabled"}>Accept</button>
                <button type="button" id="back-button">Back</button>
              </div>
            </form>`
          : `<div class="row"><button type="button" id="back-button">Back</button></div>`
      }
      ${
        state.status === "partial_decrypted" &&
        state.fanOutResponse?.remote_state === "partial_decrypted"
          ? `<button type="button" id="retry-failed-button" ${
              state.busy ? "disabled" : ""
            }>Retry failed</button>`
          : ""
      }
      ${
        state.fanOutResponse
          ? `<div class="panel stack">
              ${state.fanOutResponse.targets
                .map(
                  (target) => `<div class="row" data-testid="fanout-target">
                    <code>${escapeHtml(target.node_id)}</code>
                    <span class="${target.error_code ? "status danger" : "status"}">${escapeHtml(
                      target.remote_state ?? "pending",
                    )}</span>
                  </div>`,
                )
                .join("")}
            </div>`
          : ""
      }
    </section>
  `;

  root.querySelector("#back-button")?.addEventListener("click", () => {
    clearRetryPlaintext(state);
    deps.location.assign(backTarget);
  });
  root.querySelector("#verify-link")?.addEventListener("click", () => {
    if (manifestUrl) {
      deps.window.open(manifestUrl, "_blank", "noopener,noreferrer");
    }
  });
  root.querySelector("#verified-checkbox")?.addEventListener("change", (event) => {
    if (!state.integrity || state.integrity.optOut) return;
    const checkbox = event.currentTarget as HTMLInputElement;
    const verifiedAt = checkbox.checked
      ? writeVerificationSession(
          deps,
          state.integrity.fingerprintSha384Hex,
          state.integrity.runtimeConfig,
        )
      : null;
    state.integrity = { ...state.integrity, verifiedAt };
    render(root, state, deps);
  });
  root.querySelector("#accept-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    void submitCredential(root, state, deps);
  });
  root.querySelector("#retry-failed-button")?.addEventListener("click", () => {
    void retryFailedFanOutNodes(root, state, deps);
  });
}

async function initializeIntegrity(
  root: HTMLElement,
  state: PageState,
  deps: CredentialAcceptDeps,
): Promise<void> {
  try {
    const [runtimeConfig, fingerprintSha384Hex] = await Promise.all([
      fetchRuntimeConfig(deps),
      computeLoadedScriptFingerprint(deps),
    ]);

    let optOut = false;
    if (state.route.fanOut) {
      const pubkeys = await apiFetch<FanOutPendingCredentialPubkeysResponse>(
        deps,
        `/nodes/credentials/pending/${encodeURIComponent(state.route.pendingId)}/fan-out/pubkeys`,
      );
      optOut = Boolean(pubkeys.integrity_verification_opt_out);
    } else {
      const pubkey = await fetchPubkeyWithBackoff(deps, state.route);
      optOut = Boolean(pubkey?.integrity_verification_opt_out);
    }

    const session = readVerificationSession(deps, fingerprintSha384Hex, runtimeConfig);
    state.integrity = {
      runtimeConfig,
      fingerprintSha384Hex,
      shortFingerprint: fingerprintSha384Hex.slice(0, 12),
      optOut,
      verifiedAt: session?.verified_at ?? null,
    };
    state.formVisible = true;
  } catch (err) {
    state.status = "error";
    state.errorMessage =
      err instanceof Error ? err.message : "Failed to initialize release integrity.";
  }
  render(root, state, deps);
}

async function submitCredential(
  root: HTMLElement,
  state: PageState,
  deps: CredentialAcceptDeps,
): Promise<void> {
  clearRetryPlaintext(state);
  const input = deps.document.getElementById("credential-secret") as HTMLInputElement | null;
  const secret = input?.value ?? "";
  const plaintext = new TextEncoder().encode(secret);
  if (input) input.value = "";

  state.errorMessage = null;
  state.ciphertextResponse = null;
  state.fanOutResponse = null;
  render(root, state, deps);

  if (plaintext.length === 0) {
    plaintext.fill(0);
    state.errorMessage = "Credential value is required.";
    render(root, state, deps);
    return;
  }
  if (plaintext.length > MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE) {
    plaintext.fill(0);
    state.errorMessage = `Credential value must be ${String(MAX_REMOTE_CREDENTIAL_PLAINTEXT_SIZE)} bytes or less.`;
    render(root, state, deps);
    return;
  }

  state.busy = true;
  try {
    const integrity_verification = createIntegrityVerification(state, deps);

    if (state.route.fanOut) {
      await runFanOutCiphertextFlow(root, state, deps, plaintext, {
        route: state.route,
        pending: () => fetchFanOutStatus(deps, state.route),
        targetNodeIds: null,
        integrityVerification: integrity_verification,
      });
      return;
    }

    state.status = "waiting_pubkey";
    render(root, state, deps);
    const pubkey = await fetchPubkeyWithBackoff(deps, state.route);
    if (!pubkey) {
      state.status = "legacy_fallback";
      return;
    }

    state.status = "encrypting";
    render(root, state, deps);
    const pending = await fetchPendingMetadata(deps, state.route);
    const context = buildRciContext({
      node_id: pubkey.node_id,
      pending_credential_id: pubkey.pending_id,
      service_slug: pubkey.service_slug,
      injection_method: pending.injection_method,
      field_name: pending.field_name,
      target_url: pending.target_url ?? null,
      version: pubkey.version,
    });
    const envelope: CiphertextEnvelope = encrypt(plaintext, pubkey.node_pubkey, context);

    state.status = "posting";
    render(root, state, deps);
    const postResult = await postJson<NodePendingCredentialCiphertextResponse>(
      deps,
      `/nodes/${encodeURIComponent(pubkey.node_id)}/credentials/pending/${encodeURIComponent(pubkey.pending_id)}/ciphertext`,
      {
        ...envelope,
        integrity_verification,
      },
    );
    state.ciphertextResponse = postResult;
    const directTerminal = statusFromRemoteState(postResult.remote_state);
    if (directTerminal) {
      state.status = directTerminal;
      return;
    }
    state.status = "polling";
    render(root, state, deps);
    state.status = await pollTerminalState(deps, state.route);
  } catch (err) {
    state.status = "error";
    state.errorMessage =
      err instanceof Error ? err.message : "Failed to accept credential.";
  } finally {
    plaintext.fill(0);
    state.busy = false;
    render(root, state, deps);
  }
}

async function retryFailedFanOutNodes(
  root: HTMLElement,
  state: PageState,
  deps: CredentialAcceptDeps,
): Promise<void> {
  const fanOutResponse = state.fanOutResponse;
  if (
    !state.route.fanOut ||
    !fanOutResponse ||
    fanOutResponse.remote_state !== "partial_decrypted"
  ) {
    return;
  }

  const plaintext = state.retryPlaintext;
  if (!plaintext || plaintext.length === 0) {
    state.status = "error";
    state.errorMessage =
      "Credential plaintext is no longer available. Enter the credential again to retry.";
    render(root, state, deps);
    return;
  }

  const retryTargetIds = failedFanOutTargetIds(fanOutResponse);
  if (retryTargetIds.size === 0) {
    state.status = "error";
    state.errorMessage = "No failed fan-out targets are available to retry.";
    clearRetryPlaintext(state);
    render(root, state, deps);
    return;
  }

  const retryRoute: RouteInfo = {
    ...state.route,
    pendingId: fanOutResponse.fanout_id,
  };

  state.busy = true;
  state.errorMessage = null;
  state.ciphertextResponse = null;
  try {
    const integrity_verification = createIntegrityVerification(state, deps);

    state.status = "posting";
    render(root, state, deps);
    const retry = await postJson<FanOutPendingCredentialResponse>(
      deps,
      `/nodes/credentials/pending/${encodeURIComponent(fanOutResponse.fanout_id)}/fan-out/retry-failed`,
      {
        fan_out_revision: fanOutResponse.fan_out_revision,
      },
    );
    state.fanOutResponse = retry;

    await runFanOutCiphertextFlow(root, state, deps, plaintext, {
      route: retryRoute,
      pending: () => retry,
      targetNodeIds: retryTargetIds,
      integrityVerification: integrity_verification,
    });
  } catch (err) {
    state.status = "error";
    state.errorMessage =
      err instanceof Error ? err.message : "Failed to retry fan-out targets.";
  } finally {
    if (state.status !== "partial_decrypted") {
      clearRetryPlaintext(state);
    }
    state.busy = false;
    render(root, state, deps);
  }
}

export function bootCredentialAcceptPage(
  root: HTMLElement,
  partialDeps: Partial<CredentialAcceptDeps> = {},
): void {
  const deps: CredentialAcceptDeps = {
    fetch: partialDeps.fetch ?? window.fetch.bind(window),
    location: partialDeps.location ?? window.location,
    window: partialDeps.window ?? window,
    document: partialDeps.document ?? document,
    storage: partialDeps.storage ?? window.sessionStorage,
    now: partialDeps.now ?? (() => Date.now()),
    delay: partialDeps.delay ?? defaultDelay,
  };

  const route = parseCredentialAcceptRoute(deps.location.pathname);
  const state: PageState = {
    route,
    integrity: null,
    status: "idle",
    errorMessage: null,
    formVisible: false,
    busy: false,
    fanOutResponse: null,
    ciphertextResponse: null,
    retryPlaintext: null,
  };
  render(root, state, deps);
  void initializeIntegrity(root, state, deps);
}
