import assert from "node:assert/strict";
import { test } from "node:test";
import {
  groupCreateSchema,
  groupFeedbackSchema,
  groupIdSchema,
  groupMessagesPageSchema,
  groupPendingActionSchema,
  groupPostRequestSchema,
  groupSchema,
  groupUpdateSchema,
  type GroupCreate,
} from "./groupContracts";
import { createGroupsApi } from "../../lib/api/groupsTransport";
import type { RequestOptions } from "../../lib/api/authenticatedTransport";

const ID = "nyxg-0123456789abcdef0123456789abcdef";
const OTHER = "nyxg-1123456789abcdef0123456789abcdef";
const USER = "1785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const ACK = "e8aa62de-1c12-4ba0-b90b-92b99290d35d";
const CONVERSATION = "nyxa-0123456789abcdef0123456789abcdef";
const NOW = "2026-10-15T08:00:00Z";
const group = {
  id: ID,
  name: "Research",
  members: [{ id: "nyxb-lead", name: "nyxbot", kind: "nyxbot" }],
  lead_agent_id: "nyxb-lead",
  created_at: NOW,
};
const message = {
  id: ACK,
  seq: 3,
  role: "user",
  text: "hello",
  created_at: NOW,
};

test("group IDs use nyxg plus 32 hex characters, not UUID or arbitrary paths", () => {
  assert.equal(groupIdSchema.parse(ID), ID);
  for (const invalid of [
    USER,
    "nyxg-short",
    `${ID}/messages`,
    `${ID}?before_seq=1`,
  ]) {
    assert.equal(groupIdSchema.safeParse(invalid).success, false);
  }
  const parsed = groupSchema.parse(group);
  assert.deepEqual(parsed.working_agent_ids, []);
  assert.equal(parsed.members[0]?.destroyed, false);
  assert.equal(parsed.last_message_at, null);
  assert.equal(
    groupSchema.safeParse({ ...group, your_role: "owner" }).success,
    false,
  );
});

test("creation has a strict backend body and counts Unicode name characters", () => {
  const input = {
    name: ` ${"🪴".repeat(60)} `,
    member_agent_ids: ["nyxb-lead"],
  };
  assert.equal(groupCreateSchema.parse(input).name, "🪴".repeat(60));
  assert.equal(
    groupCreateSchema.safeParse({ ...input, name: "🪴".repeat(61) }).success,
    false,
  );
  assert.equal(
    groupCreateSchema.safeParse({ ...input, lead_agent_id: "nyxb-lead" })
      .success,
    false,
  );
  assert.equal(
    groupCreateSchema.safeParse({ ...input, participant_user_ids: [USER] })
      .success,
    false,
  );
  assert.equal(
    groupCreateSchema.safeParse({ ...input, member_agent_ids: [] }).success,
    false,
  );
  assert.equal(
    groupCreateSchema.safeParse({ ...input, member_agent_ids: ["a", "a"] })
      .success,
    false,
  );
  assert.equal(
    groupCreateSchema.safeParse({
      ...input,
      member_agent_ids: Array.from({ length: 9 }, (_, i) => `a${i}`),
    }).success,
    false,
  );
  assert.equal(
    groupCreateSchema.safeParse({
      ...input,
      org: USER,
      participant_user_ids: [],
    }).success,
    true,
  );
});

test("settings enforce leave-only updates and live lead shape without accepting create-only org", () => {
  assert.deepEqual(groupUpdateSchema.parse({ leave: true }), { leave: true });
  assert.equal(
    groupUpdateSchema.safeParse({ leave: true, name: "New" }).success,
    false,
  );
  assert.equal(groupUpdateSchema.safeParse({ org: USER }).success, false);
  assert.equal(
    groupUpdateSchema.safeParse({ participant_user_ids: [] }).success,
    false,
  );
  assert.equal(
    groupUpdateSchema.safeParse({ lead_agent_id: "b", member_agent_ids: ["a"] })
      .success,
    false,
  );
  assert.equal(
    groupUpdateSchema.safeParse({ lead_agent_id: "a", member_agent_ids: ["a"] })
      .success,
    true,
  );
});

test("posts validate attachments and explicit recipients before HTTP admission", () => {
  assert.equal(groupPostRequestSchema.safeParse({ text: " " }).success, false);
  assert.deepEqual(
    groupPostRequestSchema.parse({ text: " ", attachment_ids: [ACK] }),
    { text: "", attachment_ids: [ACK] },
  );
  assert.equal(
    groupPostRequestSchema.safeParse({ text: "hello", recipient_agent_ids: [] })
      .success,
    false,
  );
  assert.equal(
    groupPostRequestSchema.safeParse({
      text: "hello",
      recipient_agent_ids: ["a", "a"],
    }).success,
    false,
  );
  assert.equal(
    groupPostRequestSchema.safeParse({
      text: "hello",
      attachment_ids: Array(11).fill(ACK),
    }).success,
    false,
  );
  assert.equal(
    groupPostRequestSchema.safeParse({
      text: "hello",
      conversation_id: CONVERSATION,
    }).success,
    false,
  );
});

test("message pages reject invalid roles, cursors and pending confirmation shapes", () => {
  const page = { group, messages: [message] };
  assert.equal(groupMessagesPageSchema.parse(page).before_seq, null);
  for (const before_seq of [0, -2, 1.2, "5"])
    assert.equal(
      groupMessagesPageSchema.safeParse({ ...page, before_seq }).success,
      false,
    );
  assert.equal(
    groupMessagesPageSchema.safeParse({
      ...page,
      messages: [{ ...message, role: "assistant" }],
    }).success,
    false,
  );
  assert.equal(
    groupPendingActionSchema.safeParse({
      conversation_id: CONVERSATION,
      summary: "Check",
      expires_at: NOW,
      confirm_phrase: "yes 12345",
    }).success,
    false,
  );
});

test("transport preserves group paths, cursor/abort options and empty delete response", async () => {
  const calls: { path: string; options?: RequestOptions }[] = [];
  const signal = new AbortController().signal;
  const api = createGroupsApi(async (path, options) => {
    calls.push({ path, options });
    if (options?.method === "DELETE") return null;
    if (path.includes("/messages"))
      return { group, messages: [message], before_seq: 3 };
    return {
      ...group,
      owner: { type: "org", id: USER, name: "Organization" },
      participants: [],
      your_role: "creator",
    };
  });
  assert.equal((await api.messages(ID, 3, signal)).group.id, ID);
  assert.equal(
    calls[0]?.path,
    `/assistant/nyxagent/groups/${ID}/messages?limit=50&before_seq=3`,
  );
  assert.equal(calls[0]?.options?.signal, signal);
  await assert.rejects(api.messages(ID, -1));
  assert.equal(calls.length, 1);
  assert.equal(await api.remove(ID), undefined);
  assert.deepEqual((await api.leave(ID)).participants, []);
  assert.deepEqual(calls[2]?.options, {
    method: "PATCH",
    body: { leave: true },
  });
});

test("transport refuses invalid outgoing bodies and cross-group responses", async () => {
  let calls = 0;
  const api = createGroupsApi(async () => {
    calls += 1;
    return {
      ...group,
      id: OTHER,
      group: { ...group, id: OTHER },
      messages: [],
    };
  });
  await assert.rejects(
    api.create({
      name: "X",
      member_agent_ids: ["a"],
      lead_agent_id: "a",
    } as GroupCreate),
  );
  await assert.rejects(api.post(ID, { text: "hi", recipient_agent_ids: [] }));
  assert.equal(calls, 0);
  await assert.rejects(api.detail(ID), /different crew/);
  await assert.rejects(api.update(ID, { name: "X" }), /different crew/);
  await assert.rejects(api.messages(ID), /different crew/);
});

test("identity reads personal capability flags with safe defaults", async () => {
  let response: unknown = { id: USER };
  const api = createGroupsApi(async () => response);
  assert.deepEqual(await api.identity(), { id: USER, enabled_features: [] });
  response = {
    id: USER,
    capabilities: { enabled_features: ["assistant:org-agents"] },
  };
  assert.deepEqual(await api.features(), ["assistant:org-agents"]);
});

test("org decisions use approval requests and accept a successful rejection response", async () => {
  const calls: { path: string; options?: RequestOptions }[] = [];
  const api = createGroupsApi(async (path, options) => {
    calls.push({ path, options });
    return { id: ACK, status: "rejected", decided_at: null };
  });
  const action = groupPendingActionSchema.parse({
    conversation_id: CONVERSATION,
    approval_request_id: ACK,
    summary: "Connect",
    expires_at: NOW,
  });
  const result = await api.decide(action, "deny");
  assert.equal(result.status, "rejected");
  assert.deepEqual(calls, [
    {
      path: `/approvals/requests/${ACK}/decide`,
      options: { method: "POST", body: { approved: false } },
    },
  ]);
  await assert.rejects(
    api.decide({ ...action, authored_skill: true }, "allow"),
    /Review/,
  );
  assert.equal(calls.length, 1);
  const mismatched = createGroupsApi(async () => ({
    id: USER,
    status: "approved",
    decided_at: NOW,
  }));
  await assert.rejects(mismatched.decide(action, "allow"), /mismatch/);
});

test("acknowledgement decisions use their conversation resource and reject a mismatched card", async () => {
  const calls: { path: string; options?: RequestOptions }[] = [];
  let response = {
    id: ACK,
    kind: "action",
    status: "denied",
    summary: "Connect",
    service_slug: null,
    service_name: null,
    tool_name: null,
    created_at: NOW,
    expires_at: NOW,
  };
  const api = createGroupsApi(async (path, options) => {
    calls.push({ path, options });
    return response;
  });
  const action = groupPendingActionSchema.parse({
    conversation_id: CONVERSATION,
    acknowledgement_id: ACK,
    summary: "Connect",
    expires_at: NOW,
  });
  await api.decide(action, "deny");
  assert.deepEqual(calls[0], {
    path: `/assistant/nyxagent/conversations/${CONVERSATION}/acknowledgements/${ACK}`,
    options: { method: "POST", body: { decision: "deny" } },
  });
  response = { ...response, id: USER };
  await assert.rejects(api.decide(action, "deny"), /mismatch/);
});

test("history still accepts server feedback metadata without native mutation controls", () => {
  assert.deepEqual(
    groupFeedbackSchema.parse({ vote: "up", comment: null, updated_at: NOW }),
    { vote: "up", comment: null, updated_at: NOW },
  );
  assert.equal(
    groupFeedbackSchema.safeParse({
      vote: "other",
      comment: null,
      updated_at: NOW,
    }).success,
    false,
  );
});
