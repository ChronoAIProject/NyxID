#!/usr/bin/env node

/**
 * Environment and command contract for the fixed AC-2 probe:
 * - AC2_NYXID_BASE_URL must be loopback HTTP without credentials, query, or fragment.
 * - AC2_LOCAL_VERSION is a public Git SHA or conservative deployment label.
 * - AC2_IDENTITY_ASSERTION, AC2_BRIDGE_BEARER, and
 *   AC2_RESTRICTED_DELEGATED_BEARER are secrets.
 * - AC2_GATEWAY_MODEL must be a public label.
 * - AC2_ACK_LOCAL_POSTS must equal "ac2-local-only".
 * - AC2_PRODUCTION_USER_ID is the required expected operator UUID for both
 *   production commands. It has no committed default.
 * - AC2_PRODUCTION_BASE_URL defaults to https://nyx-api.chrono-ai.fun and may
 *   override that target with another HTTPS origin.
 * - AC2_AEVATAR_CLIENT_ID defaults to the registered production Aevatar client
 *   UUID and may override the client used for the consent read.
 * - Stdout contains allowlisted receipt JSON only. Redirect it to the evidence file.
 * - `production-read <token-file>` performs the fixed allowlisted GETs against
 *   the configured production origin and checks the configured operator user.
 * - `production-chat <token-file>` additionally requires
 *   AC2_ACK_PRODUCTION_CHAT="post-one-chat-and-delete". The command posts one
 *   typed turn, deletes only its new conversation, and confirms deletion.
 * - The default command is `seven-row`; it never selects a production command.
 */

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const AEVATAR_PINNED_VERSION = "e5bba2e9719ad5132004b882744caa3875db1123";
const MAX_RESPONSE_BYTES = 64 * 1024;
const REQUEST_TIMEOUT_MS = 15_000;
const LOCAL_ACKNOWLEDGEMENT = "ac2-local-only";
const PRODUCTION_CHAT_ACKNOWLEDGEMENT = "post-one-chat-and-delete";
const DEFAULT_PRODUCTION_BASE_URL = "https://nyx-api.chrono-ai.fun";
const DEFAULT_AEVATAR_CLIENT_ID = "a6ff2946-f02f-4c35-8203-1ec46132b660";
const SAFE_PUBLIC_LABEL = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const SAFE_CODE = /^[A-Za-z][A-Za-z0-9_-]{0,127}$/;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const SECRET_PATTERNS = [
  /eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}/i,
  /nyx_[A-Za-z0-9_-]+/i,
  new RegExp(`${["nyxid", "ag"].join("_")}_+[A-Za-z0-9_-]+`, "i"),
  /authorization\s*:\s*(?:bearer\s+)?\S+/i,
  /(?:set-)?cookie\s*:\s*\S+/i,
];
const ROW_IDS = [
  "identity_only",
  "identity_plus_capability_bearer",
  "identity_plus_delegation_header",
  "replayed_identity_jti",
  "bridge_bearer_mcp_config",
  "restricted_bearer_llm_gateway",
  "restricted_bearer_proxy_slug",
];

export function decodeJwtClaimSummary(token) {
  if (typeof token !== "string" || token.split(".").length !== 3) {
    throw new Error("invalid_jwt_shape");
  }

  let payload;
  try {
    payload = JSON.parse(Buffer.from(token.split(".")[1], "base64url").toString("utf8"));
  } catch {
    throw new Error("invalid_jwt_payload");
  }
  if (!isPlainObject(payload)) {
    throw new Error("invalid_jwt_payload");
  }

  const scopeNames = normalizeScopes(payload.scope ?? payload.scopes);
  const expiryWindowSeconds =
    Number.isSafeInteger(payload.iat) &&
    Number.isSafeInteger(payload.exp) &&
    payload.exp >= payload.iat
      ? payload.exp - payload.iat
      : null;

  return {
    delegated: payload.delegated === true,
    actor_present:
      isPlainObject(payload.act) &&
      typeof payload.act.sub === "string" &&
      payload.act.sub.length > 0,
    client_id_present:
      typeof payload.client_id === "string" && payload.client_id.length > 0,
    allow_all_services:
      typeof payload.allow_all_services === "boolean" ? payload.allow_all_services : null,
    allowed_service_ids_count: Array.isArray(payload.allowed_service_ids)
      ? payload.allowed_service_ids.length
      : 0,
    resource_count: Array.isArray(payload.resources) ? payload.resources.length : 0,
    scope_names: scopeNames,
    expiry_window_seconds: expiryWindowSeconds,
  };
}

export function sanitizeReceipt(candidate) {
  if (!isPlainObject(candidate) || !isPlainObject(candidate.claim_summary)) {
    throw new Error("invalid_receipt");
  }

  return {
    row_id: requireSafeString(candidate.row_id),
    surface: requireSafeString(candidate.surface),
    deployment_version: requireSafeString(candidate.deployment_version),
    request_shape: requireSafeString(candidate.request_shape),
    http_status: nullableStatus(candidate.http_status),
    upstream_code: nullableSafeCode(candidate.upstream_code),
    claim_summary: sanitizeClaimSummary(candidate.claim_summary),
    verdict: requireSafeString(candidate.verdict),
    observation_kind: requireSafeString(candidate.observation_kind),
  };
}

export function serializeReceipts(rows) {
  if (!Array.isArray(rows)) {
    throw new Error("invalid_receipts");
  }
  const sanitized = rows.map(sanitizeReceipt);
  const serialized = `${JSON.stringify(sanitized, null, 2)}\n`;
  if (SECRET_PATTERNS.some((pattern) => pattern.test(serialized))) {
    throw new Error("secret_barrier_triggered");
  }
  return serialized;
}

export function extractSafeUpstreamCode(value) {
  if (!isPlainObject(value)) {
    return null;
  }
  const direct = nullableSafeCodeOrNull(value.code);
  if (direct !== null) {
    return direct;
  }
  if (isPlainObject(value.error)) {
    return nullableSafeCodeOrNull(value.error.code);
  }
  return null;
}

export async function runSevenRowProbe({
  env = process.env,
  fetchImpl = globalThis.fetch,
} = {}) {
  const inputs = loadSevenRowInputs(env);
  const sourceRows = buildSourceRows(inputs);
  const localRows = [];

  for (const spec of localRequestSpecs(inputs)) {
    localRows.push(await observeLocalRow(spec, inputs, fetchImpl));
  }

  return [...sourceRows, ...localRows];
}

export async function runProductionReadProbe({
  token,
  env = process.env,
  fetchImpl = globalThis.fetch,
} = {}) {
  const config = loadProductionProbeConfig(env);
  requireSecret(token);
  const tokenSummary = decodeJwtClaimSummary(token);
  const context = { config, token, fetchImpl };
  const specs = [
    ["me", "/api/v1/users/me"],
    ["consents", "/api/v1/users/me/consents"],
    [
      "aevatar_consent_authorization",
      `/api/v1/users/me/consents/${config.aevatarClientId}/authorization`,
    ],
    ["keys", "/api/v1/keys"],
    ["assistant_actions", "/api/v1/assistant/actions"],
    ["assistant_readiness", "/api/v1/assistant/readiness"],
    ["mcp_config", "/api/v1/mcp/config"],
    ["assistant_conversations", "/api/v1/assistant/conversations"],
  ];
  const observations = {};
  const bodies = {};
  for (const [name, path] of specs) {
    const { response, body } = await productionJsonRequest(context, path);
    observations[name] = {
      status: response.status,
      upstream_code: extractSafeUpstreamCode(body),
      item_count: countKnownItems(body),
    };
    bodies[name] = body;
  }

  const meId = isPlainObject(bodies.me) ? bodies.me.id ?? bodies.me.user_id : null;
  const authorization = bodies.aevatar_consent_authorization;
  return {
    surface: "production-read-only",
    nyxid_base: config.baseUrl.origin,
    aevatar_source_version: AEVATAR_PINNED_VERSION,
    expected_user_matched: meId === config.expectedUserId,
    token_claim_summary: tokenSummary,
    aevatar_authorized_service_count: countNamedArray(authorization, [
      "allowed_service_ids",
      "service_ids",
      "services",
    ]),
    observations,
  };
}

export async function runProductionChatProbe({
  token,
  env = process.env,
  fetchImpl = globalThis.fetch,
} = {}) {
  if (env.AC2_ACK_PRODUCTION_CHAT !== PRODUCTION_CHAT_ACKNOWLEDGEMENT) {
    throw new Error("production_chat_not_acknowledged");
  }
  const config = loadProductionProbeConfig(env);
  requireSecret(token);
  const context = { config, token, fetchImpl };

  const meRead = await productionJsonRequest(context, "/api/v1/users/me");
  if (
    meRead.response.status !== 200 ||
    !isPlainObject(meRead.body) ||
    meRead.body.id !== config.expectedUserId
  ) {
    throw new Error("production_user_mismatch");
  }

  const keysRead = await productionJsonRequest(context, "/api/v1/keys");
  if (keysRead.response.status !== 200) {
    throw new Error("production_keys_unavailable");
  }
  const selectedService = selectConnectedService(keysRead.body);
  const beforeRead = await productionJsonRequest(context, "/api/v1/assistant/conversations");
  if (beforeRead.response.status !== 200) {
    throw new Error("production_conversations_unavailable");
  }
  const beforeIds = collectConversationIds(beforeRead.body);

  const chatResponse = await productionFetch(context, "/api/v1/assistant/chat", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "text/event-stream",
      "x-nyxid-debug-upstream": "1",
    },
    body: JSON.stringify({
      type: "text",
      prompt: `Use the already connected service ${selectedService.slug} and report whether it is available.`,
      clientRequestId: `ac2-production-${Date.now()}`,
    }),
  });
  let chatBytes = Buffer.alloc(0);
  let chatReadError = null;
  try {
    chatBytes = await readBoundedBytes(chatResponse, 8 * 1024 * 1024);
  } catch (error) {
    chatReadError = error;
  }
  const chatText = chatBytes.toString("utf8");

  const afterRead = await productionJsonRequest(context, "/api/v1/assistant/conversations");
  if (afterRead.response.status !== 200) {
    throw new Error("production_conversations_unavailable");
  }
  const afterIds = collectConversationIds(afterRead.body);
  const newIds = [...afterIds].filter((id) => !beforeIds.has(id));
  if (newIds.length !== 1) {
    throw new Error("production_conversation_identity_ambiguous");
  }
  const conversationId = newIds[0];
  const streamedIds = collectConversationIds(chatText);
  let resultError = chatReadError;
  if (
    resultError === null &&
    (streamedIds.size > 1 || (streamedIds.size === 1 && !streamedIds.has(conversationId)))
  ) {
    resultError = new Error("production_conversation_identity_ambiguous");
  }

  let deleteResponse;
  let historyConfirmation;
  let stateConfirmation;
  try {
    deleteResponse = await productionFetch(
      context,
      `/api/v1/assistant/conversations/${conversationId}`,
      { method: "DELETE" },
    );
    try {
      await readBoundedBytes(deleteResponse, 256 * 1024);
    } catch {
      // The typed reads below are the authoritative deletion confirmation.
    }
    historyConfirmation = await productionJsonRequest(
      context,
      `/api/v1/assistant/conversations/${conversationId}`,
    );
    stateConfirmation = await productionJsonRequest(
      context,
      `/api/v1/assistant/conversations/${conversationId}/state`,
    );
  } catch {
    throw new Error("production_conversation_cleanup_failed");
  }
  const deletionConfirmed =
    historyConfirmation.response.status === 404 && stateConfirmation.response.status === 404;
  if (!deletionConfirmed) {
    throw new Error("production_conversation_cleanup_failed");
  }
  if (resultError !== null) {
    throw resultError;
  }

  const recognizedClassifications = [
    "USER_SERVICE_ACCESS_REQUIRED",
    "SourceStale",
    "INVENTORY_INVALID",
    "unsupported-action",
  ].filter((classification) => chatText.includes(classification));

  return {
    surface: "production-single-chat",
    aevatar_source_version: AEVATAR_PINNED_VERSION,
    selected_service_id: selectedService.id,
    chat_status: chatResponse.status,
    chat_response_bytes: chatBytes.length,
    recognized_classifications: recognizedClassifications,
    created_conversation_id: conversationId,
    delete_status: deleteResponse.status,
    history_confirmation_status: historyConfirmation.response.status,
    history_confirmation_code: extractSafeUpstreamCode(historyConfirmation.body),
    state_confirmation_status: stateConfirmation.response.status,
    state_confirmation_code: extractSafeUpstreamCode(stateConfirmation.body),
    deletion_confirmed: deletionConfirmed,
  };
}

export function serializeSafeEvidence(value) {
  const serialized = `${JSON.stringify(value, null, 2)}\n`;
  if (SECRET_PATTERNS.some((pattern) => pattern.test(serialized))) {
    throw new Error("secret_barrier_triggered");
  }
  return serialized;
}

function loadSevenRowInputs(env) {
  if (env.AC2_ACK_LOCAL_POSTS !== LOCAL_ACKNOWLEDGEMENT) {
    throw new Error("local_posts_not_acknowledged");
  }
  const baseUrl = assertLoopbackTarget(env.AC2_NYXID_BASE_URL);
  const deploymentVersion = requirePublicLabel(
    env.AC2_LOCAL_VERSION,
    "invalid_local_version",
  );
  const gatewayModel = requirePublicLabel(env.AC2_GATEWAY_MODEL, "invalid_gateway_model");
  const identityAssertion = requireSecret(env.AC2_IDENTITY_ASSERTION);
  const bridgeBearer = requireSecret(env.AC2_BRIDGE_BEARER);
  const restrictedBearer = requireSecret(env.AC2_RESTRICTED_DELEGATED_BEARER);

  return {
    baseUrl,
    deploymentVersion,
    gatewayModel,
    identityAssertion,
    bridgeBearer,
    restrictedBearer,
    identitySummary: decodeJwtClaimSummary(identityAssertion),
    bridgeSummary: decodeJwtClaimSummary(bridgeBearer),
    restrictedSummary: decodeJwtClaimSummary(restrictedBearer),
  };
}

function assertLoopbackTarget(rawUrl) {
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new Error("local_base_url_invalid");
  }
  const loopbackHosts = new Set(["127.0.0.1", "[::1]", "localhost"]);
  if (
    url.protocol !== "http:" ||
    !loopbackHosts.has(url.hostname) ||
    url.username !== "" ||
    url.password !== "" ||
    url.search !== "" ||
    url.hash !== ""
  ) {
    throw new Error("local_base_url_not_loopback");
  }
  url.pathname = url.pathname.replace(/\/$/, "");
  return url;
}

function loadProductionProbeConfig(env) {
  const expectedUserId = requireUuid(
    env.AC2_PRODUCTION_USER_ID,
    "production_user_id_missing",
    "production_user_id_invalid",
  );
  const aevatarClientId = requireUuid(
    env.AC2_AEVATAR_CLIENT_ID ?? DEFAULT_AEVATAR_CLIENT_ID,
    "aevatar_client_id_missing",
    "aevatar_client_id_invalid",
  );
  let baseUrl;
  try {
    baseUrl = new URL(env.AC2_PRODUCTION_BASE_URL ?? DEFAULT_PRODUCTION_BASE_URL);
  } catch {
    throw new Error("production_base_url_invalid");
  }
  if (
    baseUrl.protocol !== "https:" ||
    baseUrl.username !== "" ||
    baseUrl.password !== "" ||
    baseUrl.pathname !== "/" ||
    baseUrl.search !== "" ||
    baseUrl.hash !== ""
  ) {
    throw new Error("production_base_url_invalid");
  }
  return Object.freeze({ baseUrl, expectedUserId, aevatarClientId });
}

function requireUuid(value, missingCode, invalidCode) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(missingCode);
  }
  if (!UUID.test(value)) {
    throw new Error(invalidCode);
  }
  return value;
}

function buildSourceRows(inputs) {
  const source = "pinned-aevatar-source";
  const observation = "source-proven-not-runtime-observed";
  const common = {
    surface: source,
    deployment_version: AEVATAR_PINNED_VERSION,
    http_status: null,
    upstream_code: null,
    observation_kind: observation,
  };

  return [
    sanitizeReceipt({
      ...common,
      row_id: ROW_IDS[0],
      request_shape: "identity assertion only",
      claim_summary: inputs.identitySummary,
      verdict: "execution_capability_required",
    }),
    sanitizeReceipt({
      ...common,
      row_id: ROW_IDS[1],
      request_shape: "identity assertion plus capability bearer",
      claim_summary: inputs.restrictedSummary,
      verdict: "capability_selected_from_bearer",
    }),
    sanitizeReceipt({
      ...common,
      row_id: ROW_IDS[2],
      request_shape: "identity assertion plus delegation header",
      claim_summary: inputs.bridgeSummary,
      verdict: "capability_selected_from_delegation_header",
    }),
    sanitizeReceipt({
      ...common,
      row_id: ROW_IDS[3],
      request_shape: "replayed identity assertion identifier",
      http_status: 401,
      upstream_code: "identity_assertion_replayed",
      claim_summary: inputs.identitySummary,
      verdict: "replay_rejected",
    }),
  ];
}

function localRequestSpecs(inputs) {
  const completionBody = JSON.stringify({
    model: inputs.gatewayModel,
    messages: [{ role: "user", content: "AC-2 local authority reachability probe" }],
    stream: false,
  });
  return [
    {
      rowId: ROW_IDS[4],
      requestShape: "bridge bearer reads REST MCP config",
      path: "/api/v1/mcp/config",
      init: { method: "GET", headers: bearerHeaders(inputs.bridgeBearer) },
      claimSummary: inputs.bridgeSummary,
      verdictForStatus: (status) => (status === 403 ? "route_denied" : "unexpected_result"),
    },
    {
      rowId: ROW_IDS[5],
      requestShape: "restricted delegated bearer posts LLM gateway completion",
      path: "/api/v1/llm/gateway/v1/chat/completions",
      init: {
        method: "POST",
        headers: jsonBearerHeaders(inputs.restrictedBearer),
        body: completionBody,
      },
      claimSummary: inputs.restrictedSummary,
      verdictForStatus: (status) =>
        status >= 200 && status < 300 ? "callback_reachable" : "unexpected_result",
    },
    {
      rowId: ROW_IDS[6],
      requestShape: "restricted delegated bearer posts platform proxy slug",
      path: "/api/v1/proxy/s/chrono-llm-public",
      init: {
        method: "POST",
        headers: jsonBearerHeaders(inputs.restrictedBearer),
        body: completionBody,
      },
      claimSummary: inputs.restrictedSummary,
      verdictForStatus: (status) =>
        status === 403 ? "restricted_platform_row_denied" : "unexpected_result",
    },
  ];
}

async function observeLocalRow(spec, inputs, fetchImpl) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  let response;
  try {
    response = await fetchImpl(new URL(spec.path, inputs.baseUrl), {
      ...spec.init,
      redirect: "manual",
      signal: controller.signal,
    });
  } catch {
    throw new Error("network_failure");
  } finally {
    clearTimeout(timeout);
  }

  const body = await readBoundedJson(response);
  return sanitizeReceipt({
    row_id: spec.rowId,
    surface: "local-nyxid-runtime",
    deployment_version: inputs.deploymentVersion,
    request_shape: spec.requestShape,
    http_status: response.status,
    upstream_code: extractSafeUpstreamCode(body),
    claim_summary: spec.claimSummary,
    verdict: spec.verdictForStatus(response.status),
    observation_kind: "runtime-observed",
  });
}

async function readBoundedJson(response) {
  const declaredLength = Number(response.headers.get("content-length"));
  if (Number.isFinite(declaredLength) && declaredLength > MAX_RESPONSE_BYTES) {
    throw new Error("response_too_large");
  }
  if (response.body === null) {
    return null;
  }

  const reader = response.body.getReader();
  const chunks = [];
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      totalBytes += value.byteLength;
      if (totalBytes > MAX_RESPONSE_BYTES) {
        await reader.cancel();
        throw new Error("response_too_large");
      }
      chunks.push(value);
    }
  } catch (error) {
    if (error instanceof Error && error.message === "response_too_large") {
      throw error;
    }
    throw new Error("response_read_failed");
  }

  if (totalBytes === 0) {
    return null;
  }
  try {
    const bytes = Buffer.concat(chunks.map((chunk) => Buffer.from(chunk)));
    return JSON.parse(bytes.toString("utf8"));
  } catch {
    return null;
  }
}

async function productionJsonRequest(context, path) {
  const response = await productionFetch(context, path, { method: "GET" });
  const bytes = await readBoundedBytes(response, 4 * 1024 * 1024);
  let body = null;
  if (bytes.length > 0) {
    try {
      body = JSON.parse(bytes.toString("utf8"));
    } catch {
      body = null;
    }
  }
  return { response, body };
}

async function productionFetch(context, path, init) {
  let response;
  try {
    response = await context.fetchImpl(new URL(path, context.config.baseUrl), {
      ...init,
      headers: {
        ...init.headers,
        authorization: `Bearer ${context.token}`,
      },
      redirect: "manual",
    });
  } catch {
    throw new Error("network_failure");
  }
  return response;
}

async function readBoundedBytes(response, limit) {
  const declaredLength = Number(response.headers.get("content-length"));
  if (Number.isFinite(declaredLength) && declaredLength > limit) {
    throw new Error("response_too_large");
  }
  if (response.body === null) {
    return Buffer.alloc(0);
  }
  const reader = response.body.getReader();
  const chunks = [];
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      totalBytes += value.byteLength;
      if (totalBytes > limit) {
        await reader.cancel();
        throw new Error("response_too_large");
      }
      chunks.push(Buffer.from(value));
    }
  } catch (error) {
    if (error instanceof Error && error.message === "response_too_large") {
      throw error;
    }
    throw new Error("response_read_failed");
  }
  return Buffer.concat(chunks);
}

function selectConnectedService(body) {
  const keys = isPlainObject(body) && Array.isArray(body.keys) ? body.keys : [];
  const selected = keys.find(
    (key) =>
      isPlainObject(key) &&
      key.is_active !== false &&
      typeof key.id === "string" &&
      /^[A-Za-z0-9_-]{1,128}$/.test(key.id) &&
      typeof key.slug === "string" &&
      /^[a-z0-9][a-z0-9-]{0,99}$/.test(key.slug),
  );
  if (!selected) {
    throw new Error("production_connected_service_missing");
  }
  return { id: selected.id, slug: selected.slug };
}

function collectConversationIds(value) {
  const ids = new Set();
  if (typeof value === "string") {
    for (const match of value.matchAll(/(?:nyxid-chat-[A-Za-z0-9_-]+|chatc-[A-Za-z0-9_-]+)/g)) {
      ids.add(match[0]);
    }
    return ids;
  }
  visitJson(value, (key, candidate) => {
    if (
      ["id", "conversationId", "conversation_id"].includes(key) &&
      typeof candidate === "string" &&
      /^(?:nyxid-chat-|chatc-)[A-Za-z0-9_-]+$/.test(candidate)
    ) {
      ids.add(candidate);
    }
  });
  return ids;
}

function visitJson(value, visitor) {
  if (Array.isArray(value)) {
    for (const item of value) {
      visitJson(item, visitor);
    }
    return;
  }
  if (!isPlainObject(value)) {
    return;
  }
  for (const [key, candidate] of Object.entries(value)) {
    visitor(key, candidate);
    visitJson(candidate, visitor);
  }
}

function countKnownItems(value) {
  if (Array.isArray(value)) {
    return value.length;
  }
  return countNamedArray(value, [
    "actions",
    "consents",
    "conversations",
    "items",
    "keys",
    "services",
  ]);
}

function countNamedArray(value, names) {
  if (!isPlainObject(value)) {
    return 0;
  }
  for (const name of names) {
    if (Array.isArray(value[name])) {
      return value[name].length;
    }
  }
  return 0;
}

function sanitizeClaimSummary(summary) {
  return {
    delegated: summary.delegated === true,
    actor_present: summary.actor_present === true,
    client_id_present: summary.client_id_present === true,
    allow_all_services:
      typeof summary.allow_all_services === "boolean" ? summary.allow_all_services : null,
    allowed_service_ids_count: nonnegativeSafeInteger(summary.allowed_service_ids_count),
    resource_count: nonnegativeSafeInteger(summary.resource_count),
    scope_names: Array.isArray(summary.scope_names)
      ? [...new Set(summary.scope_names.filter(isSafeScope))].sort()
      : [],
    expiry_window_seconds:
      summary.expiry_window_seconds === null
        ? null
        : nonnegativeSafeInteger(summary.expiry_window_seconds),
  };
}

function normalizeScopes(value) {
  const candidates = Array.isArray(value)
    ? value
    : typeof value === "string"
      ? value.split(/\s+/)
      : [];
  return [...new Set(candidates.filter(isSafeScope).map((scope) => scope.trim()))].sort();
}

function isSafeScope(value) {
  return typeof value === "string" && /^[A-Za-z0-9:*._/-]{1,128}$/.test(value.trim());
}

function bearerHeaders(secret) {
  return { authorization: `Bearer ${secret}` };
}

function jsonBearerHeaders(secret) {
  return {
    ...bearerHeaders(secret),
    "content-type": "application/json",
  };
}

function nullableStatus(value) {
  if (value === null) {
    return null;
  }
  if (Number.isInteger(value) && value >= 100 && value <= 599) {
    return value;
  }
  throw new Error("invalid_receipt_status");
}

function nullableSafeCode(value) {
  if (value === null) {
    return null;
  }
  const safe = nullableSafeCodeOrNull(value);
  if (safe === null) {
    throw new Error("invalid_receipt_code");
  }
  return safe;
}

function nullableSafeCodeOrNull(value) {
  if (Number.isSafeInteger(value) && value >= 0 && value <= 999_999) {
    return value;
  }
  if (typeof value === "string" && SAFE_CODE.test(value)) {
    return value;
  }
  return null;
}

function nonnegativeSafeInteger(value) {
  if (Number.isSafeInteger(value) && value >= 0) {
    return value;
  }
  throw new Error("invalid_receipt_count");
}

function requireSafeString(value) {
  if (typeof value !== "string" || value.length === 0 || value.length > 256) {
    throw new Error("invalid_receipt_string");
  }
  return value;
}

function requirePublicLabel(value, errorCode) {
  if (typeof value !== "string" || !SAFE_PUBLIC_LABEL.test(value)) {
    throw new Error(errorCode);
  }
  return value;
}

function requireSecret(value) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error("missing_probe_secret");
  }
  return value;
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isMainModule() {
  return process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href;
}

async function runCli() {
  const command = process.argv[2] ?? "seven-row";
  if (command === "seven-row") {
    return serializeReceipts(await runSevenRowProbe());
  }
  if (command === "production-read" || command === "production-chat") {
    const tokenFile = process.argv[3];
    if (typeof tokenFile !== "string" || tokenFile.length === 0) {
      throw new Error("production_token_file_missing");
    }
    let token;
    try {
      token = (await readFile(tokenFile, "utf8")).trim();
    } catch {
      throw new Error("production_token_file_unreadable");
    }
    const result = command === "production-read"
      ? await runProductionReadProbe({ token })
      : await runProductionChatProbe({ token });
    return serializeSafeEvidence(result);
  }
  throw new Error("unknown_probe_command");
}

if (isMainModule()) {
  runCli()
    .then((output) => process.stdout.write(output))
    .catch((error) => {
      const safeCodes = new Set([
        "invalid_gateway_model",
        "invalid_jwt_payload",
        "invalid_jwt_shape",
        "invalid_local_version",
        "local_base_url_invalid",
        "local_base_url_not_loopback",
        "local_posts_not_acknowledged",
        "missing_probe_secret",
        "network_failure",
        "aevatar_client_id_invalid",
        "aevatar_client_id_missing",
        "production_base_url_invalid",
        "production_chat_not_acknowledged",
        "production_connected_service_missing",
        "production_conversation_cleanup_failed",
        "production_conversation_identity_ambiguous",
        "production_conversations_unavailable",
        "production_keys_unavailable",
        "production_token_file_missing",
        "production_token_file_unreadable",
        "production_user_mismatch",
        "production_user_id_invalid",
        "production_user_id_missing",
        "response_read_failed",
        "response_too_large",
        "secret_barrier_triggered",
        "unknown_probe_command",
      ]);
      const code = error instanceof Error && safeCodes.has(error.message)
        ? error.message
        : "probe_failed";
      process.stderr.write(`${code}\n`);
      process.exitCode = 1;
    });
}
