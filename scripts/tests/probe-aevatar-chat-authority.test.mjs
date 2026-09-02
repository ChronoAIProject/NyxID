import assert from "node:assert/strict";
import test from "node:test";

import {
  decodeJwtClaimSummary,
  extractSafeUpstreamCode,
  runProductionChatProbe,
  runSevenRowProbe,
  sanitizeReceipt,
  serializeReceipts,
} from "../probe-aevatar-chat-authority.mjs";

const ROW_IDS = [
  "identity_only",
  "identity_plus_capability_bearer",
  "identity_plus_delegation_header",
  "replayed_identity_jti",
  "bridge_bearer_mcp_config",
  "restricted_bearer_llm_gateway",
  "restricted_bearer_proxy_slug",
];

function jwtWithPayload(payload) {
  const header = Buffer.from(JSON.stringify({ alg: "RS256", typ: "JWT" }))
    .toString("base64url");
  const body = Buffer.from(JSON.stringify(payload)).toString("base64url");
  return `${header}.${body}.test-signature`;
}

function safeRow(rowId) {
  return {
    row_id: rowId,
    surface: "pinned-source",
    deployment_version: "e5bba2e9719ad5132004b882744caa3875db1123",
    request_shape: "identity assertion only",
    http_status: null,
    upstream_code: null,
    claim_summary: {
      delegated: false,
      actor_present: true,
      client_id_present: false,
      allow_all_services: null,
      allowed_service_ids_count: 0,
      resource_count: 0,
      scope_names: ["account:read"],
      expiry_window_seconds: 60,
    },
    verdict: "capability_required",
    observation_kind: "source-proven-not-runtime-observed",
  };
}

test("JWT parsing returns only the allowlisted summary", () => {
  const token = jwtWithPayload({
    sub: "user-secret",
    act: { sub: "actor-secret" },
    client_id: "client-secret",
    jti: "replay-secret",
    delegated: true,
    allow_all_services: false,
    allowed_service_ids: ["service-a", "service-b"],
    resources: ["resource-a"],
    scope: "proxy:read account:read proxy:read",
    iat: 1_000,
    exp: 1_300,
    unknown_secret: "must-not-survive",
  });

  assert.deepEqual(decodeJwtClaimSummary(token), {
    delegated: true,
    actor_present: true,
    client_id_present: true,
    allow_all_services: false,
    allowed_service_ids_count: 2,
    resource_count: 1,
    scope_names: ["account:read", "proxy:read"],
    expiry_window_seconds: 300,
  });
});

test("JWT parsing normalizes absent and wrongly typed claims", () => {
  const token = jwtWithPayload({
    act: { sub: 42 },
    client_id: [],
    delegated: "true",
    allow_all_services: "false",
    allowed_service_ids: "service-a",
    resources: null,
    scope: [" proxy:write ", "account:read", "proxy:write", 7],
    iat: "1000",
    exp: 1300,
  });

  assert.deepEqual(decodeJwtClaimSummary(token), {
    delegated: false,
    actor_present: false,
    client_id_present: false,
    allow_all_services: null,
    allowed_service_ids_count: 0,
    resource_count: 0,
    scope_names: ["account:read", "proxy:write"],
    expiry_window_seconds: null,
  });
});

test("malformed JWT failures never include token fragments", () => {
  const secretFragment = "sensitive-fragment";

  assert.throws(
    () => decodeJwtClaimSummary(`header.${secretFragment}.signature`),
    (error) => {
      assert.equal(error.message, "invalid_jwt_payload");
      assert.equal(error.message.includes(secretFragment), false);
      return true;
    },
  );
  assert.throws(
    () => decodeJwtClaimSummary("not-a-jwt"),
    { message: "invalid_jwt_shape" },
  );
});

test("receipt sanitization reconstructs the schema and drops unknown fields", () => {
  const candidate = {
    ...safeRow("identity_only"),
    authorization: "must-not-survive",
    cookie: "must-not-survive",
    refresh_token: "must-not-survive",
    jti: "must-not-survive",
    claim_summary: {
      ...safeRow("identity_only").claim_summary,
      sub: "must-not-survive",
      resources: ["must-not-survive"],
    },
  };

  const sanitized = sanitizeReceipt(candidate);

  assert.deepEqual(Object.keys(sanitized), [
    "row_id",
    "surface",
    "deployment_version",
    "request_shape",
    "http_status",
    "upstream_code",
    "claim_summary",
    "verdict",
    "observation_kind",
  ]);
  assert.deepEqual(Object.keys(sanitized.claim_summary), [
    "delegated",
    "actor_present",
    "client_id_present",
    "allow_all_services",
    "allowed_service_ids_count",
    "resource_count",
    "scope_names",
    "expiry_window_seconds",
  ]);
  assert.equal(JSON.stringify(sanitized).includes("must-not-survive"), false);
});

test("serialization rejects secret-shaped content in allowed string fields", () => {
  const secretShapes = [
    ["eyJhbGciOiJSUzI1NiJ9", "eyJzdWIiOiJ4In0", "signature"].join("."),
    ["nyx", "secret_material"].join("_"),
    ["nyxid", "ag", "secret_material"].join("_"),
    ["Authorization", "Bearer secret-material"].join(": "),
    ["Cookie", "session=secret-material"].join(": "),
    ["Set-Cookie", "session=secret-material"].join(": "),
  ];

  for (const secretShape of secretShapes) {
    const row = safeRow("identity_only");
    row.verdict = secretShape;
    assert.throws(() => serializeReceipts([row]), {
      message: "secret_barrier_triggered",
    });
  }
});

test("safe upstream-code parsing never returns response prose", () => {
  assert.equal(extractSafeUpstreamCode({ code: 8001 }), 8001);
  assert.equal(
    extractSafeUpstreamCode({ error: { code: "identity_assertion_replayed" } }),
    "identity_assertion_replayed",
  );
  assert.equal(extractSafeUpstreamCode({ code: "safe-code_12" }), "safe-code_12");
  assert.equal(extractSafeUpstreamCode({ code: "contains spaces and prose" }), null);
  assert.equal(extractSafeUpstreamCode({ message: "sensitive response body" }), null);
  assert.equal(extractSafeUpstreamCode(null), null);
});

test("missing local acknowledgement performs no requests", async () => {
  let fetchCalls = 0;
  await assert.rejects(
    runSevenRowProbe({
      env: localEnv({ AC2_ACK_LOCAL_POSTS: undefined }),
      fetchImpl: async () => {
        fetchCalls += 1;
        throw new Error("must not run");
      },
    }),
    { message: "local_posts_not_acknowledged" },
  );
  assert.equal(fetchCalls, 0);
});

test("non-loopback targets perform no requests", async () => {
  let fetchCalls = 0;
  await assert.rejects(
    runSevenRowProbe({
      env: localEnv({ AC2_NYXID_BASE_URL: "https://nyx-api.example.test" }),
      fetchImpl: async () => {
        fetchCalls += 1;
        throw new Error("must not run");
      },
    }),
    { message: "local_base_url_not_loopback" },
  );
  assert.equal(fetchCalls, 0);
});

test("network failures use a fixed error without exposing rejection messages", async () => {
  const secretFragment = "network-secret-fragment";
  await assert.rejects(
    runSevenRowProbe({
      env: localEnv(),
      fetchImpl: async () => {
        throw new Error(secretFragment);
      },
    }),
    (error) => {
      assert.equal(error.message, "network_failure");
      assert.equal(error.message.includes(secretFragment), false);
      return true;
    },
  );
});

test("the fixed probe emits exactly seven rows in stable order", async () => {
  const responses = [
    jsonResponse(403, { code: 1003 }),
    jsonResponse(200, { id: "discarded" }),
    jsonResponse(403, { code: 1003, message: "discarded" }),
  ];
  const rows = await runSevenRowProbe({
    env: localEnv(),
    fetchImpl: async () => responses.shift(),
  });
  const output = serializeReceipts(rows);
  const parsed = JSON.parse(output);

  assert.equal(parsed.length, 7);
  assert.deepEqual(parsed.map((row) => row.row_id), ROW_IDS);
  assert.equal(output.endsWith("\n"), true);
  assert.equal(output.includes("discarded"), false);
});

test("production chat requires an explicit one-write acknowledgement", async () => {
  let fetchCalls = 0;
  await assert.rejects(
    runProductionChatProbe({
      token: jwtWithPayload({ iat: 100, exp: 160 }),
      env: {},
      fetchImpl: async () => {
        fetchCalls += 1;
        throw new Error("must not run");
      },
    }),
    { message: "production_chat_not_acknowledged" },
  );
  assert.equal(fetchCalls, 0);
});

test("production chat verifies the fixed operator before any write", async () => {
  const writeMethods = [];

  await assert.rejects(
    runProductionChatProbe({
      token: jwtWithPayload({ iat: 100, exp: 160 }),
      env: { AC2_ACK_PRODUCTION_CHAT: "post-one-chat-and-delete" },
      fetchImpl: async (_url, init) => {
        if (init.method !== "GET") {
          writeMethods.push(init.method);
        }
        return jsonResponse(200, { id: "different-user" });
      },
    }),
    { message: "production_user_mismatch" },
  );
  assert.deepEqual(writeMethods, []);
});

test("production chat never deletes an id that existed before the turn", async () => {
  const existingId = "nyxid-chat-existing";
  const deletePaths = [];

  await assert.rejects(
    runProductionChatProbe({
      token: jwtWithPayload({ iat: 100, exp: 160 }),
      env: { AC2_ACK_PRODUCTION_CHAT: "post-one-chat-and-delete" },
      fetchImpl: async (url, init) => {
        if (url.pathname === "/api/v1/users/me") {
          return jsonResponse(200, {
            id: "10ddeeb3-9e40-4ee7-b58c-dbd9af615c3b",
          });
        }
        if (url.pathname === "/api/v1/keys") {
          return jsonResponse(200, {
            keys: [{ id: "service-id", slug: "service-slug", is_active: true }],
          });
        }
        if (url.pathname === "/api/v1/assistant/conversations") {
          return jsonResponse(200, { conversations: [{ id: existingId }] });
        }
        if (url.pathname === "/api/v1/assistant/chat") {
          return new Response(`data: ${JSON.stringify({ conversationId: existingId })}\n\n`, {
            status: 200,
            headers: { "content-type": "text/event-stream" },
          });
        }
        if (init.method === "DELETE") {
          deletePaths.push(url.pathname);
          return jsonResponse(200, { ok: true });
        }
        return jsonResponse(404, { code: 1000 });
      },
    }),
    { message: "production_conversation_identity_ambiguous" },
  );
  assert.deepEqual(deletePaths, []);
});

test("production chat cleans up its new conversation after a body read failure", async () => {
  const conversationId = "nyxid-chat-new";
  const observedPaths = [];
  let conversationReads = 0;

  await assert.rejects(
    runProductionChatProbe({
      token: jwtWithPayload({ iat: 100, exp: 160 }),
      env: { AC2_ACK_PRODUCTION_CHAT: "post-one-chat-and-delete" },
      fetchImpl: async (url, init) => {
        observedPaths.push(`${init.method} ${url.pathname}`);
        if (url.pathname === "/api/v1/users/me") {
          return jsonResponse(200, {
            id: "10ddeeb3-9e40-4ee7-b58c-dbd9af615c3b",
          });
        }
        if (url.pathname === "/api/v1/keys") {
          return jsonResponse(200, {
            keys: [{ id: "service-id", slug: "service-slug", is_active: true }],
          });
        }
        if (url.pathname === "/api/v1/assistant/conversations") {
          conversationReads += 1;
          return conversationReads === 1
            ? jsonResponse(200, { conversations: [] })
            : jsonResponse(200, { conversations: [{ id: conversationId }] });
        }
        if (url.pathname === "/api/v1/assistant/chat") {
          return new Response(
            new ReadableStream({
              start(controller) {
                controller.error(new Error("sensitive stream failure"));
              },
            }),
            { status: 200, headers: { "content-type": "text/event-stream" } },
          );
        }
        if (
          init.method === "DELETE" &&
          url.pathname === `/api/v1/assistant/conversations/${conversationId}`
        ) {
          return jsonResponse(200, { ok: true });
        }
        if (
          init.method === "GET" &&
          (url.pathname === `/api/v1/assistant/conversations/${conversationId}` ||
            url.pathname === `/api/v1/assistant/conversations/${conversationId}/state`)
        ) {
          return jsonResponse(404, { code: 1000 });
        }
        return jsonResponse(404, { code: 1000 });
      },
    }),
    { message: "response_read_failed" },
  );
  assert.deepEqual(observedPaths.slice(-3), [
    `DELETE /api/v1/assistant/conversations/${conversationId}`,
    `GET /api/v1/assistant/conversations/${conversationId}`,
    `GET /api/v1/assistant/conversations/${conversationId}/state`,
  ]);
});

test("production chat reports a fixed error when cleanup is not confirmed", async () => {
  const conversationId = "nyxid-chat-new";
  let conversationReads = 0;

  await assert.rejects(
    runProductionChatProbe({
      token: jwtWithPayload({ iat: 100, exp: 160 }),
      env: { AC2_ACK_PRODUCTION_CHAT: "post-one-chat-and-delete" },
      fetchImpl: async (url, init) => {
        if (url.pathname === "/api/v1/users/me") {
          return jsonResponse(200, {
            id: "10ddeeb3-9e40-4ee7-b58c-dbd9af615c3b",
          });
        }
        if (url.pathname === "/api/v1/keys") {
          return jsonResponse(200, {
            keys: [{ id: "service-id", slug: "service-slug", is_active: true }],
          });
        }
        if (url.pathname === "/api/v1/assistant/conversations") {
          conversationReads += 1;
          return conversationReads === 1
            ? jsonResponse(200, { conversations: [] })
            : jsonResponse(200, { conversations: [{ id: conversationId }] });
        }
        if (url.pathname === "/api/v1/assistant/chat") {
          return new Response("data: done\n\n", { status: 200 });
        }
        if (init.method === "DELETE") {
          return jsonResponse(200, { ok: true });
        }
        return jsonResponse(200, { id: conversationId });
      },
    }),
    { message: "production_conversation_cleanup_failed" },
  );
});

function localEnv(overrides = {}) {
  const identity = jwtWithPayload({
    act: { sub: "actor" },
    iat: 100,
    exp: 160,
  });
  const bridge = jwtWithPayload({
    delegated: true,
    act: { sub: "actor" },
    allow_all_services: true,
    allowed_service_ids: [],
    iat: 100,
    exp: 160,
  });
  const restricted = jwtWithPayload({
    delegated: true,
    act: { sub: "actor" },
    allow_all_services: false,
    allowed_service_ids: ["service"],
    iat: 100,
    exp: 160,
  });

  return {
    AC2_NYXID_BASE_URL: "http://127.0.0.1:3187",
    AC2_LOCAL_VERSION: "3fbae40a7b9526afdc97b3fdc005c1a543dabaaf",
    AC2_IDENTITY_ASSERTION: identity,
    AC2_BRIDGE_BEARER: bridge,
    AC2_RESTRICTED_DELEGATED_BEARER: restricted,
    AC2_GATEWAY_MODEL: "local-probe-model",
    AC2_ACK_LOCAL_POSTS: "ac2-local-only",
    ...overrides,
  };
}

function jsonResponse(status, body) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}
