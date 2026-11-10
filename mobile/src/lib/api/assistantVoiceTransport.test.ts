import assert from "node:assert/strict";
import test from "node:test";
import AbortControllerPolyfill from "abort-controller";
import {
  assistantVoiceSessionsPath,
  assistantVoiceStreamPath,
  createAssistantVoiceApi,
  trustedVoiceOrigin,
} from "./assistantVoiceTransport";
import type { RequestOptions } from "./authenticatedTransport";
import {
  voiceStartedSchema,
  type VoicePreferences,
} from "../../features/assistant/voice/voiceContracts";
import {
  AssistantVoiceClient,
  type VoiceMediaAdapter,
} from "../../features/assistant/voice/AssistantVoiceClient";

const thread = `nyxa-${"a".repeat(32)}`;
const sessionId = "11111111-1111-4111-8111-111111111111";
const requestId = "22222222-2222-4222-8222-222222222222";
const origin = "https://nyx.example";
const root = `/assistant/nyxagent/conversations/${thread}`;
const preferences: VoicePreferences = {
  service_id: requestId,
  connection_id: null,
  key_source: "platform",
  model: "gpt-live-1",
  voice: "marin",
  input_mode: "push_to_talk",
  language: null,
  notify_on_completion: false,
};
const session = {
  created_at: "2026-10-16T04:00:00Z",
  closed_at: null,
  id: sessionId,
  generation: 1,
  state: "active",
  muted: false,
  input_muted: false,
  end_requested: false,
  control_revision: 0,
  observed_seconds: 3,
  reserved_until: 15,
  final_usage_confirmed: false,
  end_reason: null,
  idle_warning: false,
  resumable: true,
};
function fixture() {
  const calls: { path: string; options?: RequestOptions }[] = [];
  let result: unknown;
  let failure: Error | undefined;
  const api = createAssistantVoiceApi(async (path, options) => {
    calls.push({ path, options });
    if (failure) throw failure;
    return result;
  }, `${origin}/`);
  return {
    api,
    calls,
    reply(value: unknown) {
      result = value;
    },
    fail(error: Error) {
      failure = error;
    },
  };
}

test("trusted voice origin accepts configured origins and rejects paths or credentials", () => {
  assert.equal(trustedVoiceOrigin(`${origin}/`), origin);
  assert.equal(
    trustedVoiceOrigin("http://localhost:5173"),
    "http://localhost:5173",
  );
  for (const value of [
    "",
    "null",
    "file:///voice.html",
    "javascript:alert(1)",
    "ftp://nyx.example",
    "https://user:pass@nyx.example",
    `${origin}/assistant`,
    `${origin}?token=secret`,
    `${origin}#voice`,
    ` ${origin}`,
    `${origin}\n`,
    "https:\\nyx.example",
    "https://nyx.example/../assistant",
    "https://nyx.example/assistant/..",
    "https://nyx.example/%2e%2e/",
    "https:nyx.example",
    "https://@nyx.example",
  ]) {
    assert.throws(() => trustedVoiceOrigin(value), /trusted frontend origin/);
    assert.throws(
      () => createAssistantVoiceApi(async () => null, value),
      /trusted frontend origin/,
    );
  }
});

test("voice path helpers validate thread and session identity before interpolation", () => {
  assert.equal(assistantVoiceSessionsPath(thread), `${root}/voice-sessions`);
  assert.equal(
    assistantVoiceStreamPath(thread, sessionId),
    `${root}/voice-sessions/${sessionId}/stream`,
  );
  for (const id of [
    "../agents",
    `${thread}?id=other`,
    "group-123",
    "direct-123",
  ])
    assert.throws(() => assistantVoiceSessionsPath(id));
  for (const id of [
    "../active",
    `${sessionId}/stream`,
    `${sessionId}?secret=x`,
  ])
    assert.throws(() => assistantVoiceStreamPath(thread, id));
});

test("identity features default to disabled and use server-resolved capabilities", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({ id: requestId });
  assert.deepEqual(await f.api.features(signal), []);
  f.reply({
    id: requestId,
    capabilities: { enabled_features: ["assistant:voice"] },
  });
  assert.deepEqual(await f.api.features(signal), ["assistant:voice"]);
  assert.equal((await f.api.identity()).id, requestId);
  assert.deepEqual(f.calls[0], { path: "/users/me", options: { signal } });
  f.reply({
    id: "untrusted-account",
    capabilities: { enabled_features: ["assistant:voice"] },
  });
  await assert.rejects(f.api.features());
});

test("account active metadata binds the session to a valid conversation", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({ session, conversation_id: thread });
  assert.deepEqual(await f.api.accountActive(thread, signal), {
    session,
    conversation_id: thread,
  });
  assert.deepEqual(f.calls, [
    { path: "/assistant/nyxagent/voice/active", options: { signal } },
  ]);
  for (const result of [
    { session, conversation_id: null },
    { session: null, conversation_id: thread },
    { session, conversation_id: "../other" },
  ]) {
    f.reply(result);
    await assert.rejects(f.api.accountActive(thread));
  }
});

test("old-server discovery finds another chat through bounded metadata reads", async () => {
  const ids = Array.from(
    { length: 15 },
    (_, index) => `nyxa-${String(index + 1).padStart(32, "0")}`,
  );
  const calls: string[] = [];
  let concurrent = 0;
  let maximum = 0;
  const signal = new AbortController().signal;
  const api = createAssistantVoiceApi(async (path, options) => {
    calls.push(path);
    assert.equal(options?.signal, signal);
    if (path === "/assistant/nyxagent/voice/active") throw { statusCode: 404 };
    if (path.endsWith("?limit=10"))
      return { conversations: ids.map((id) => ({ id })), next_cursor: "more" };
    concurrent++;
    maximum = Math.max(maximum, concurrent);
    await Promise.resolve();
    concurrent--;
    return { session: path.includes(ids[9]!) ? session : null };
  }, origin);
  assert.deepEqual(await api.accountActive(thread, signal), {
    session,
    conversation_id: ids[9],
  });
  assert.equal(maximum, 2);
  assert.equal(calls.filter((path) => path.endsWith("/active")).length, 12);
  assert.equal(
    calls.some((path) => path.includes(ids[10]!)),
    false,
  );
  assert.ok(calls.every((path) => !path.endsWith("/history")));
});

test("an old-server scan that finds no live call in recent chats clears recovery even with more chats", async () => {
  // Accounts with more than ten chats always have a next page; treating that
  // as permanently unknown blocked voice. Server admission still fences Start.
  let posts = 0;
  const api = createAssistantVoiceApi(async (path, options) => {
    if (options?.method === "POST") posts++;
    if (path === "/assistant/nyxagent/voice/active") throw { statusCode: 405 };
    if (path.endsWith("?limit=10"))
      return {
        conversations: Array.from({ length: 10 }, (_, index) => ({
          id: `nyxa-${String(index + 100).padStart(32, "0")}`,
        })),
        next_cursor: "more",
      };
    return { session: null };
  }, origin);
  assert.deepEqual(await api.accountActive(thread), {
    session: null,
    conversation_id: null,
  });
  assert.equal(posts, 0);
});

test("account authorization failure or cancellation never triggers recent-chat scanning", async () => {
  for (const failure of [
    { statusCode: 401 },
    { statusCode: 403 },
    Object.assign(new Error("cancelled"), { name: "AbortError" }),
  ]) {
    const calls: string[] = [];
    const api = createAssistantVoiceApi(async (path) => {
      calls.push(path);
      throw failure;
    }, origin);
    await assert.rejects(
      api.accountActive(thread),
      (error) => error === failure,
    );
    assert.deepEqual(calls, ["/assistant/nyxagent/voice/active"]);
  }
});

test("cancelling legacy discovery stops the scan before another conversation batch", async () => {
  const controller = new AbortController();
  const calls: string[] = [];
  const api = createAssistantVoiceApi(async (path) => {
    calls.push(path);
    if (path === "/assistant/nyxagent/voice/active") throw { statusCode: 404 };
    if (path.endsWith("?limit=10")) {
      controller.abort();
      return {
        conversations: [{ id: `nyxa-${"b".repeat(32)}` }],
        next_cursor: null,
      };
    }
    return { session: null };
  }, origin);
  await assert.rejects(api.accountActive(thread, controller.signal), {
    name: "AbortError",
  });
  assert.equal(calls.length, 3);
});

test("account recovery supports React Native's AbortSignal polyfill without throwIfAborted", async () => {
  const controller = new AbortControllerPolyfill();
  const signal = controller.signal as unknown as AbortSignal;
  assert.equal(typeof signal.throwIfAborted, "undefined");
  const f = fixture();
  f.reply({ session: null, conversation_id: null });
  assert.deepEqual(await f.api.accountActive(thread, signal), {
    session: null,
    conversation_id: null,
  });
  controller.abort();
  await assert.rejects(f.api.accountActive(thread, signal), {
    name: "AbortError",
  });
  assert.equal(f.calls.length, 1);
});

test("settings reads and saves only the voice preference patch", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({ access_mode: "nyxmode", voice: preferences });
  assert.deepEqual(await f.api.settings(signal), { voice: preferences });
  assert.deepEqual(await f.api.savePreferences(preferences, signal), {
    voice: preferences,
  });
  assert.deepEqual(f.calls, [
    { path: "/assistant/nyxagent/settings", options: { signal } },
    {
      path: "/assistant/nyxagent/settings",
      options: { method: "PUT", body: { voice: preferences }, signal },
    },
  ]);
  f.reply({ voice: null });
  assert.deepEqual(await f.api.settings(), { voice: null });
});

test("options bind a valid private NyxAgent conversation and preserve an empty catalog", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({ options: [], recording: false });
  assert.deepEqual(await f.api.options(thread, signal), {
    options: [],
    recording: false,
  });
  assert.deepEqual(f.calls, [
    {
      path: `/assistant/nyxagent/voice/options?conversation_id=${thread}`,
      options: { signal },
    },
  ]);
  await assert.rejects(f.api.options(`${thread}&conversation_id=other`));
  assert.equal(f.calls.length, 1);
});

test("Settings reads voice options without binding a conversation", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({ options: [] });
  assert.deepEqual(await f.api.options(undefined, signal), { options: [] });
  assert.deepEqual(f.calls, [
    { path: "/assistant/nyxagent/voice/options", options: { signal } },
  ]);
});

test("start sends exactly one validated request with trusted Origin and cancellation", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  const body = {
    client_request_id: requestId,
    preferences,
    sdp_offer: "v=0\r\nm=audio 9 RTP 111",
  };
  f.reply({ session, sdp_answer: "v=0\r\nm=audio 9 RTP 111" });
  const started = voiceStartedSchema.parse(
    await f.api.start(thread, body, signal),
  );
  assert.equal(started.session.id, sessionId);
  assert.deepEqual(f.calls, [
    {
      path: `${root}/voice-sessions`,
      options: { method: "POST", headers: { Origin: origin }, body, signal },
    },
  ]);
  f.fail(new Error("network outcome unknown"));
  await assert.rejects(f.api.start(thread, body), /network outcome unknown/);
  assert.equal(
    f.calls.length,
    2,
    "unknown start outcomes are never retried by transport",
  );
});

test("Grok starts use an empty offer and keep provider configuration server-owned", async () => {
  const f = fixture();
  f.reply({
    session: { ...session, state: "starting", resumable: false },
    sdp_answer: "",
  });
  const started = voiceStartedSchema.parse(
    await f.api.start(thread, {
      client_request_id: requestId,
      preferences,
      sdp_offer: "",
    }),
  );
  assert.equal(started.sdp_answer, "");
  assert.equal(started.session.resumable, false);
  assert.deepEqual(f.calls[0]?.options?.body, {
    client_request_id: requestId,
    preferences,
    sdp_offer: "",
  });
});

test("active and status read the requested session without adding a provider endpoint", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({ session: null });
  assert.deepEqual(await f.api.active(thread, signal), { session: null });
  f.reply(session);
  assert.equal((await f.api.status(thread, sessionId, signal)).id, sessionId);
  assert.deepEqual(f.calls, [
    { path: `${root}/voice-sessions/active`, options: { signal } },
    { path: `${root}/voice-sessions/${sessionId}`, options: { signal } },
  ]);
  f.reply({ ...session, id: requestId });
  await assert.rejects(
    f.api.status(thread, sessionId),
    /different voice resource/,
  );
});

test("mute/end controls retain command IDs and optimistic revisions with trusted Origin", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({ ...session, muted: true, control_revision: 1 });
  const body = {
    command_id: requestId,
    expected_revision: 0,
    action: "mute" as const,
  };
  assert.equal(
    (await f.api.control(thread, sessionId, body, signal)).muted,
    true,
  );
  assert.deepEqual(f.calls[0], {
    path: `${root}/voice-sessions/${sessionId}/control`,
    options: { method: "POST", headers: { Origin: origin }, body, signal },
  });
  f.reply({ ...session, id: requestId });
  await assert.rejects(
    f.api.control(thread, sessionId, { ...body, action: "end" }),
    /different voice resource/,
  );
});

test("voice request inspection fences IDs and Stop uses only the dedicated cancel route", async () => {
  const f = fixture();
  const signal = new AbortController().signal;
  f.reply({
    id: requestId,
    state: "claimed",
    turn_id: "turn-1",
    pending_acknowledgement_ids: [],
  });
  assert.equal((await f.api.request(thread, requestId, signal)).id, requestId);
  f.reply(null);
  await f.api.cancelRequest(thread, requestId, signal);
  assert.deepEqual(f.calls, [
    { path: `${root}/voice-requests/${requestId}`, options: { signal } },
    {
      path: `${root}/voice-requests/${requestId}/cancel`,
      options: { method: "POST", body: {}, signal },
    },
  ]);
  f.reply({
    id: sessionId,
    state: "claimed",
    turn_id: "turn-1",
    pending_acknowledgement_ids: [],
  });
  await assert.rejects(
    f.api.request(thread, requestId),
    /different voice resource/,
  );
});

test("invalid identities and mutation bodies fail before any request is sent", async () => {
  const f = fixture();
  const body = { client_request_id: requestId, preferences, sdp_offer: "" };
  await assert.rejects(f.api.start("../other-thread", body));
  await assert.rejects(f.api.status(thread, "../other-session"));
  await assert.rejects(f.api.active("nyxa-invalid"));
  await assert.rejects(f.api.cancelRequest(thread, "../other-task"));
  await assert.rejects(
    f.api.control(thread, "../other-session", {
      command_id: requestId,
      expected_revision: 0,
      action: "end",
    }),
  );
  await assert.rejects(
    f.api.start(thread, {
      ...body,
      preferences: { ...preferences, connection_id: sessionId },
    }),
  );
  await assert.rejects(
    f.api.control(thread, sessionId, {
      command_id: requestId,
      expected_revision: -1,
      action: "end",
    }),
  );
  await assert.rejects(
    f.api.savePreferences({
      ...preferences,
      model: "https://provider.example",
    }),
  );
  assert.equal(f.calls.length, 0);
});

test("raw start responses reach the controller while transport errors remain unchanged", async () => {
  const f = fixture();
  const body = { client_request_id: requestId, preferences, sdp_offer: "" };
  const malformed = {
    session: { ...session, id: "provider-id" },
    sdp_answer: "",
  };
  f.reply(malformed);
  assert.equal(await f.api.start(thread, body), malformed);
  const failure = new Error("permission revoked");
  f.fail(failure);
  await assert.rejects(
    f.api.cancelRequest(thread, requestId),
    (error) => error === failure,
  );
  assert.equal(f.calls.length, 2);
});

function integratedStart(response: unknown | (() => Promise<unknown>)) {
  const calls: { path: string; options?: RequestOptions }[] = [];
  const errors: string[] = [];
  const conflicts: unknown[] = [];
  const mediaEvents: string[] = [];
  const controlStates: {
    startAborted: boolean | undefined;
    endAborted: boolean | undefined;
  }[] = [];
  const api = createAssistantVoiceApi(async (path, options) => {
    calls.push({ path, options });
    if (path.endsWith("/active")) return { session, conversation_id: thread };
    if (path.endsWith("/control")) {
      controlStates.push({
        startAborted: calls[0]?.options?.signal?.aborted,
        endAborted: options?.signal?.aborted,
      });
      return { ...session, state: "closing", end_requested: true };
    }
    return typeof response === "function" ? response() : response;
  }, origin);
  const media: VoiceMediaAdapter = {
    prepare: async () => "offer",
    answer: async () => {
      mediaEvents.push("answer");
    },
    setInput: () => {},
    hold: () => {},
    muteSpeaker: () => {},
    resume: async () => {},
    enqueue: () => {},
    flush: () => {},
    playback: () => ({ playback_ms: 0, playback_audible: false }),
    close: () => {
      mediaEvents.push("close");
    },
    subscribe: () => () => {},
  };
  const client = new AssistantVoiceClient({
    conversationId: thread,
    api,
    media,
    randomId: () => requestId,
    createSocket: async () => {
      mediaEvents.push("socket");
      throw new Error("Invalid startup must not open controls");
    },
    onSnapshot: () => {},
    onError: (message) => errors.push(message),
    onConflict: (conflict) => conflicts.push(conflict),
  });
  return { client, calls, errors, conflicts, mediaEvents, controlStates };
}

test("real transport preserves an owned session so malformed media answers receive durable End", async () => {
  for (const sdp_answer of [undefined, 7, ""]) {
    const f = integratedStart({ session, sdp_answer });
    await f.client.start(preferences);
    await f.client.end();
    assert.equal(f.client.isClosed, true);
    assert.deepEqual(f.errors, [
      "Voice server returned an invalid session answer",
    ]);
    assert.deepEqual(f.mediaEvents, ["close"]);
    assert.equal((f.conflicts[0] as typeof session).id, sessionId);
    assert.equal((f.conflicts[0] as typeof session).state, "closing");
    assert.deepEqual(
      f.calls.map((call) => call.path),
      [`${root}/voice-sessions`, `${root}/voice-sessions/${sessionId}/control`],
    );
    assert.deepEqual(f.calls[1]!.options?.body, {
      command_id: requestId,
      expected_revision: session.control_revision,
      action: "end",
    });
    const startSignal = f.calls[0]!.options?.signal;
    const endSignal = f.calls[1]!.options?.signal;
    assert.ok(startSignal);
    assert.ok(
      endSignal,
      "durable End has an independent bounded cancellation signal",
    );
    assert.notEqual(endSignal, startSignal);
    assert.deepEqual(f.controlStates, [
      { startAborted: true, endAborted: false },
    ]);
    assert.equal(startSignal.aborted, true);
    assert.equal(
      endSignal.aborted,
      true,
      "End cleans up its own controller after settling",
    );
  }
});

test("closing during a real POST still ends a late owned session with a malformed answer", async () => {
  let resolve!: (value: unknown) => void;
  const waiting = new Promise<unknown>((yes) => {
    resolve = yes;
  });
  const f = integratedStart(() => waiting);
  const starting = f.client.start(preferences);
  for (let step = 0; step < 8 && !f.calls.length; step++)
    await Promise.resolve();
  assert.equal(f.calls.length, 1);
  await f.client.end();
  assert.equal(f.calls[0]!.options?.signal?.aborted, true);
  resolve({ session, sdp_answer: 7 });
  await starting;
  assert.deepEqual(
    f.calls.map((call) => call.path),
    [`${root}/voice-sessions`, `${root}/voice-sessions/${sessionId}/control`],
  );
  const startSignal = f.calls[0]!.options?.signal;
  const endSignal = f.calls[1]!.options?.signal;
  assert.ok(startSignal);
  assert.ok(
    endSignal,
    "a cancelled Start must not cancel its late durable End",
  );
  assert.notEqual(endSignal, startSignal);
  assert.deepEqual(f.controlStates, [
    { startAborted: true, endAborted: false },
  ]);
  assert.equal(startSignal.aborted, true);
  assert.equal(
    endSignal.aborted,
    true,
    "late End releases its own cancellation resources",
  );
  assert.deepEqual(f.mediaEvents, ["close"]);
  assert.deepEqual(f.errors, []);
  assert.equal((f.conflicts[0] as typeof session).state, "closing");
});

test("real transport never turns an invalid start identity into authority to end another active session", async () => {
  const f = integratedStart({
    session: { ...session, id: "invalid" },
    sdp_answer: 7,
  });
  await f.client.start(preferences);
  await f.client.start(preferences);
  await f.client.end();
  assert.deepEqual(
    f.calls.map((call) => call.path),
    [`${root}/voice-sessions`, "/assistant/nyxagent/voice/active"],
  );
  assert.equal(f.conflicts.length, 1);
  assert.equal((f.conflicts[0] as typeof session).id, sessionId);
  assert.deepEqual(f.mediaEvents, ["close"]);
});
