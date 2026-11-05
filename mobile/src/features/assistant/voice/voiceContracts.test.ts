import assert from "node:assert/strict";
import test from "node:test";
import {
  activeVoiceSessionSchema,
  voiceControlRequestSchema,
  voiceMetadataSchema,
  voiceOptionsSchema,
  voicePreferencesSchema,
  voiceSessionSchema,
  voiceSnapshotSchema,
  voiceStartedSchema,
  voiceStartFailureSchema,
  voiceStartRequestSchema,
} from "./voiceContracts";

const id = "11111111-1111-4111-8111-111111111111";
const connectionId = "22222222-2222-4222-8222-222222222222";
const preferences = {
  service_id: id,
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
  id,
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
};
const metadata = {
  protocol: "openai_live",
  models: [{ id: "gpt-live-1", label: "GPT Live", default: true }],
  voices: [{ id: "marin", label: "Marin" }],
  usage_source: "provider_reported",
  billing_metrics: ["voice_seconds"],
};

test("voice preferences preserve the server provider/key/microphone contract", () => {
  assert.deepEqual(voicePreferencesSchema.parse(preferences), preferences);
  const own = {
    ...preferences,
    key_source: "own",
    connection_id: connectionId,
    input_mode: "automatic",
    language: "zh-CN",
  };
  assert.deepEqual(voicePreferencesSchema.parse(own), own);
  assert.equal(
    voicePreferencesSchema.safeParse({ ...own, connection_id: null }).success,
    false,
  );
  assert.equal(
    voicePreferencesSchema.safeParse({
      ...preferences,
      connection_id: connectionId,
    }).success,
    false,
  );
});

test("voice preferences never accept provider URLs, keys or execution instructions", () => {
  for (const patch of [
    { service_id: "../another-service" },
    { model: "https://provider.example/model" },
    { voice: "voice\nheader" },
    { language: "" },
    { instructions: "execute this" },
    { api_key: "must-not-reach-media" },
  ])
    assert.equal(
      voicePreferencesSchema.safeParse({ ...preferences, ...patch }).success,
      false,
    );
});

test("metadata distinguishes OpenAI WebRTC and xAI relay without assuming entitlement", () => {
  assert.equal(voiceMetadataSchema.parse(metadata).protocol, "openai_live");
  assert.equal(
    voiceMetadataSchema.parse({
      ...metadata,
      protocol: "xai_realtime",
      usage_source: "server_measured",
    }).protocol,
    "xai_realtime",
  );
  for (const patch of [
    { usage_source: "server_measured" },
    { protocol: "openai_realtime" },
    { models: [...metadata.models, ...metadata.models] },
    { voices: [...metadata.voices, ...metadata.voices] },
    { billing_metrics: ["tokens"] },
    { billing_metrics: ["voice_seconds", "voice_seconds"] },
  ])
    assert.equal(
      voiceMetadataSchema.safeParse({ ...metadata, ...patch }).success,
      false,
    );
});

test("voice options retain availability, exact decimal tariffs and server limits", () => {
  const option = {
    service_id: id,
    connection_id: null,
    key_source: "platform",
    model: "gpt-live-1",
    model_label: "GPT Live",
    default_model: true,
    voice: metadata,
    available: false,
    unavailable_reason: "Voice is not enabled for this source",
    billing_owner: "person",
    pricing: {
      metric: "voice_seconds",
      credits_per_unit: "0.000000000001",
      sync_status: "synced",
      components: [],
    },
  };
  const result = voiceOptionsSchema.parse({
    options: [option],
    limits: {
      call_seconds: 1800,
      idle_seconds: 180,
      idle_warning_seconds: 15,
      transmitting_calls: 1,
      queued_requests: 10,
    },
    recording: false,
  });
  assert.equal(result.options[0]?.available, false);
  assert.equal(result.options[0]?.pricing?.credits_per_unit, "0.000000000001");
  assert.equal(result.limits?.call_seconds, 1800);
  assert.deepEqual(voiceOptionsSchema.parse({ options: [] }), { options: [] });
  assert.equal(
    voiceOptionsSchema.safeParse({ options: [], recording: true }).success,
    false,
  );
});

test("session decoding preserves terminal usage and backwards-compatible resumability", () => {
  assert.equal(voiceSessionSchema.parse(session).resumable, true);
  const closed = voiceSessionSchema.parse({
    ...session,
    state: "closed",
    closed_at: "2026-10-16T04:01:00Z",
    resumable: false,
    final_usage_confirmed: true,
    observed_seconds: 60,
    end_reason: "user_ended",
  });
  assert.equal(closed.resumable, false);
  assert.equal(closed.observed_seconds, 60);
  assert.equal(closed.final_usage_confirmed, true);
  assert.deepEqual(activeVoiceSessionSchema.parse({ session: null }), {
    session: null,
  });
  for (const patch of [
    { id: "provider-session-id" },
    { generation: -1 },
    { control_revision: 0.5 },
    { state: "connected" },
    { observed_seconds: -1 },
    { created_at: "yesterday" },
  ])
    assert.equal(
      voiceSessionSchema.safeParse({ ...session, ...patch }).success,
      false,
    );
});

test("snapshots use authoritative cumulative captions and retain task result references", () => {
  const result = voiceSnapshotSchema.parse({
    type: "snapshot",
    session,
    captions: [
      {
        id: "caption-1",
        speaker: "user",
        text: "查看消息",
        sealed: true,
        complete: true,
      },
      {
        id: "caption-2",
        speaker: "assistant",
        text: "正在查看",
        sealed: true,
        complete: false,
      },
    ],
    tasks: [
      {
        id: connectionId,
        turn_id: "turn-1",
        state: "awaiting_confirmation",
        pending_acknowledgement_ids: ["ack-1"],
        title: "查看关注的人的消息",
        result_message_id: "message-1",
      },
    ],
  });
  assert.equal(result.captions[1]?.complete, false);
  assert.equal(result.tasks[0]?.state, "awaiting_confirmation");
  assert.equal(result.tasks[0]?.result_message_id, "message-1");
  assert.equal(
    voiceSnapshotSchema.safeParse({ ...result, type: "provider.function_call" })
      .success,
    false,
  );
  assert.equal(
    voiceSnapshotSchema.safeParse({
      ...result,
      tasks: [{ ...result.tasks[0], id: "../other-task" }],
    }).success,
    false,
  );
});

test("start admits empty Grok SDP and bounded WebRTC SDP, never secret-bearing fields", () => {
  const request = { client_request_id: id, preferences, sdp_offer: "" };
  assert.equal(voiceStartRequestSchema.parse(request).sdp_offer, "");
  assert.equal(
    voiceStartRequestSchema
      .parse({
        ...request,
        sdp_offer: "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111",
      })
      .sdp_offer.startsWith("v=0"),
    true,
  );
  for (const patch of [
    { client_request_id: "replayed-session" },
    { sdp_offer: "x".repeat(65537) },
    { provider_session_id: "secret-session" },
    { user_id: id },
  ])
    assert.equal(
      voiceStartRequestSchema.safeParse({ ...request, ...patch }).success,
      false,
    );
  assert.equal(
    voiceStartedSchema.parse({ session, sdp_answer: "" }).sdp_answer,
    "",
  );
});

test("control commands bind revisions and reject general execution requests", () => {
  for (const action of ["mute", "unmute", "end"])
    assert.equal(
      voiceControlRequestSchema.safeParse({
        command_id: id,
        expected_revision: 3,
        action,
      }).success,
      true,
    );
  for (const patch of [
    { command_id: "not-a-uuid" },
    { expected_revision: -1 },
    { action: "run_tool" },
    { transcript: "approve everything" },
  ])
    assert.equal(
      voiceControlRequestSchema.safeParse({
        command_id: id,
        expected_revision: 0,
        action: "end",
        ...patch,
      }).success,
      false,
    );
});

test("startup failures expose bounded diagnostic identifiers only", () => {
  const failure = {
    type: "start_failed",
    error: {
      error: "voice_unavailable",
      error_code: 12501,
      message: "Voice could not connect",
      details: {
        stage: "provider_create",
        reason: "Provider declined the request",
        provider: "openai",
        provider_status: 403,
        provider_code: "model_not_found",
        provider_param: "session.model",
      },
    },
  };
  assert.equal(
    voiceStartFailureSchema.parse(failure).error.details.provider_status,
    403,
  );
  for (const patch of [
    { provider_code: "token Bearer abc" },
    { provider_param: "https://secret.example" },
    { reason: "x".repeat(401) },
    { stage: "unknown" },
  ])
    assert.equal(
      voiceStartFailureSchema.safeParse({
        ...failure,
        error: {
          ...failure.error,
          details: { ...failure.error.details, ...patch },
        },
      }).success,
      false,
    );
});
