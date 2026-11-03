import assert from "node:assert/strict";
import test from "node:test";
import {
  createCollaborationApi,
  inviteToken,
  invitationLink,
  operationPayload,
} from "./collaborationTransport";
import {
  groupSchema,
  groupCreateSchema,
  groupMessagesPageSchema,
} from "../../features/groups/groupContracts";
import {
  sharingFormSchema,
  sharingSchema,
} from "../../features/agents/sharingContracts";
import { agentServiceOperationsSchema } from "../../features/agents/operationContracts";
import type { RequestOptions } from "./authenticatedTransport";

const ID = "nyxg-0123456789abcdef0123456789abcdef";
const USER = "1785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const TOKEN = "a".repeat(64);
const NOW = "2026-11-03T01:00:00Z";
const policy = {
  version: 1,
  creator_user_id: USER,
  membership_revision: 7,
  history_disclosure: "Read all retained history",
  can_manage: true,
  your_request_ids: ["request-1"],
  transfer_to_user_id: null,
  guest_access: [
    {
      agent_id: "agent-1",
      enabled: false,
      revision: 3,
      can_enable: true,
      can_disable: false,
      lane: "owner",
    },
  ],
  proposals: [],
  invites: [],
};
const group = {
  id: ID,
  name: "Group",
  members: [
    {
      id: "agent-1",
      name: "research",
      kind: "specialist",
      shared: true,
      unavailable: false,
    },
  ],
  lead_agent_id: "agent-1",
  collaboration: policy,
  created_at: NOW,
};
const service = agentServiceOperationsSchema.parse({
  service_id: "connection-exact",
  service_slug: "github",
  service_name: "GitHub",
  revision: 8,
  all_operations: false,
  allows_explicit_rules: false,
  endpoint_ids: ["read", "write"],
  rules: [],
  inputs: { read: { value: "retained" }, write: { value: "removed" } },
  operations: [
    {
      endpoint_id: "read",
      method: "GET",
      path: "/repos/{id}",
      summary: null,
      read_only: true,
      changes_existing: false,
    },
  ],
});

function setup(response: unknown = { group, invite: null }) {
  const calls: Array<{ path: string; options?: RequestOptions }> = [];
  const api = createCollaborationApi(async (path, options) => {
    calls.push({ path, options });
    return response;
  });
  return { api, calls };
}

test("collaboration additive metadata survives parsing while legacy crew creation stays distinct", () => {
  const parsed = groupSchema.parse(group);
  assert.equal(parsed.collaboration?.membership_revision, 7);
  assert.equal(parsed.members[0]?.shared, true);
  assert.equal(
    groupSchema.parse({ ...group, collaboration: undefined }).collaboration,
    undefined,
  );
  assert.equal(
    groupSchema.safeParse({
      ...group,
      collaboration: { ...policy, version: 2 },
    }).success,
    false,
  );
  assert.equal(
    groupCreateSchema.safeParse({
      name: "Group",
      member_agent_ids: ["agent-1"],
      collaboration: true,
      org: USER,
    }).success,
    false,
  );
  assert.equal(
    groupCreateSchema.parse({
      name: "Group",
      member_agent_ids: ["agent-1"],
      collaboration: true,
    }).collaboration,
    true,
  );
  const page = groupMessagesPageSchema.parse({
    group,
    messages: [],
    collaboration_decisions: [
      {
        id: "consent-1",
        kind: "consent",
        requester_id: USER,
        agent_id: "agent-1",
        can_decide: true,
        summary: "Read issues",
        connection_id: "my-connection",
        audience: "Everyone",
      },
    ],
  });
  assert.equal(page.collaboration_decisions?.[0]?.kind, "consent");
});

test("group membership sends both revisions and checks returned identity", async () => {
  const { api, calls } = setup();
  await api.change(ID, 7, {
    action: "guest_access",
    agent_id: "agent-1",
    enabled: true,
    guest_revision: 3,
  });
  assert.deepEqual(calls[0], {
    path: `/assistant/nyxagent/groups/${ID}/membership`,
    options: {
      method: "POST",
      body: {
        expected_revision: 7,
        action: "guest_access",
        agent_id: "agent-1",
        enabled: true,
        guest_revision: 3,
      },
    },
  });
  await assert.rejects(
    setup({
      group: { ...group, id: ID.replace("0123", "1123") },
      invite: null,
    }).api.change(ID, 7, { action: "invite" }),
    /mismatch/,
  );
  await assert.rejects(api.change(ID, 0, { action: "invite" }));
});

test("invitations preview before disclosure-bearing accept without following pasted URLs", async () => {
  assert.equal(
    inviteToken(`https://nyxid.ai/assistant#group-invite=${TOKEN}`),
    TOKEN,
  );
  assert.equal(inviteToken(` ${TOKEN.toUpperCase()} `), TOKEN.toLowerCase());
  assert.equal(
    invitationLink("https://nyxid.ai/account", TOKEN),
    `https://nyxid.ai/assistant#group-invite=${TOKEN}`,
  );
  assert.equal(invitationLink("javascript:alert(1)", TOKEN), undefined);
  const preview = setup({
    group_id: ID,
    group_name: "Group",
    history_disclosure: "History",
    expires_at: NOW,
  });
  await preview.api.preview(TOKEN);
  assert.deepEqual(preview.calls[0]?.options?.body, { token: TOKEN });
  await preview.api.preview(
    `https://untrusted.invalid/assistant#group-invite=${TOKEN}`,
  );
  assert.deepEqual(preview.calls[1], {
    path: "/assistant/nyxagent/group-invites/preview",
    options: { method: "POST", body: { token: TOKEN }, signal: undefined },
  });
  const accept = setup(group);
  await accept.api.accept(TOKEN);
  assert.deepEqual(accept.calls[0]?.options?.body, {
    token: TOKEN,
    history_disclosed: true,
  });
});

test("invalid invitation formats show a human hint and preview/accept never dispatch", async () => {
  const { api, calls } = setup();
  const error = {
    name: "Error",
    message:
      "Paste the full invitation link or a 64-character invitation code.",
  };
  for (const input of [
    "",
    "   ",
    "not-an-invitation",
    "a".repeat(63),
    "a".repeat(65),
    "g".repeat(64),
    "https://untrusted.invalid/path",
    `https://nyxid.ai/assistant?group-invite=${TOKEN}`,
    "https://nyxid.ai/assistant#other=value",
    "https://nyxid.ai/assistant#group-invite=",
    `https://nyxid.ai/assistant#group-invite=${"g".repeat(64)}`,
  ]) {
    assert.throws(() => inviteToken(input), error);
    await assert.rejects(api.preview(input), error);
    await assert.rejects(api.accept(input), error);
    assert.equal(calls.length, 0);
  }
});

test("consent decisions cannot be mistaken for legacy allow/deny or owner approvals", async () => {
  const { api, calls } = setup({});
  for (const decision of ["once", "always", "skip"])
    await api.decide(ID, "consent/1", "consent", decision);
  assert.equal(calls.length, 3);
  assert.equal(
    calls[0]?.path,
    `/assistant/nyxagent/groups/${ID}/decisions/consent%2F1`,
  );
  assert.deepEqual(calls[1]?.options?.body, {
    kind: "consent",
    decision: "always",
  });
  await assert.rejects(api.decide(ID, "consent/1", "consent", "allow"));
  assert.equal(calls.length, 3);
  await api.decide(ID, "approval/1", "approval", "deny");
  assert.equal(calls[3]?.path, "/approvals/requests/approval%2F1/decide");
  assert.deepEqual(calls[3]?.options?.body, { approved: false });
  await api.cancel(ID, "request-1");
  assert.equal(
    calls[4]?.path,
    `/assistant/nyxagent/groups/${ID}/requests/request-1/cancel`,
  );
});

test("sharing supports person email and org UUID, forbids organization managers, and sends CAS version", async () => {
  const form = sharingFormSchema.parse({
    enabled: true,
    audience: [
      { subject_kind: "person", email: "a@example.com", role: "manager" },
      { subject_kind: "org", subject_id: USER, role: "user" },
    ],
    shared_access: { exact: "read" },
  });
  assert.equal(
    sharingFormSchema.safeParse({
      ...form,
      audience: [{ subject_kind: "org", subject_id: USER, role: "manager" }],
    }).success,
    false,
  );
  assert.equal(
    sharingFormSchema.safeParse({
      ...form,
      audience: [
        {
          subject_kind: "person",
          subject_id: USER,
          email: "a@example.com",
          role: "user",
        },
      ],
    }).success,
    false,
  );
  const { api, calls } = setup({});
  await api.saveSharing("agent/1", 12, form);
  assert.equal(calls[0]?.path, "/assistant/nyxagent/agents/agent%2F1/sharing");
  assert.deepEqual(calls[0]?.options?.body, { ...form, expected_version: 12 });
  const recipient = sharingSchema.parse({
    enabled: true,
    role: "user",
    version: 1,
    revision: 1,
    owner_id: USER,
  });
  assert.deepEqual(recipient.audience, []);
  assert.deepEqual(recipient.shared_access, {});
});

test("private notes and published memory use separate revision-fenced resources with Unicode bounds", async () => {
  const { api, calls } = setup({});
  await api.saveNotes("agent-1", "private-notes", 2, ["私密"]);
  await api.saveNotes("agent-1", "shared-memory", 5, ["🪴".repeat(500)]);
  assert.equal(
    calls[0]?.path,
    "/assistant/nyxagent/agents/agent-1/private-notes",
  );
  assert.deepEqual(calls[0]?.options?.body, {
    expected_revision: 2,
    notes: ["私密"],
  });
  assert.equal(
    calls[1]?.path,
    "/assistant/nyxagent/agents/agent-1/shared-memory",
  );
  await assert.rejects(
    api.saveNotes("agent-1", "private-notes", 2, ["x".repeat(501)]),
  );
  assert.equal(calls.length, 2);
});

test("operation writes preserve only retained endpoint value limits and use exact service IDs", async () => {
  const selection = {
    expected_revision: 8,
    all_operations: false,
    endpoint_ids: ["read"],
    rules: [],
  };
  const payload = operationPayload(service, selection);
  assert.deepEqual(payload.inputs, { read: { value: "retained" } });
  assert.deepEqual(
    operationPayload(service, { ...selection, all_operations: true }),
    { ...selection, all_operations: true, endpoint_ids: [], rules: [] },
  );
  const { api, calls } = setup({});
  await api.saveOperations("agent-1", true, "exact/connection", payload);
  assert.equal(
    calls[0]?.path,
    "/assistant/nyxagent/agents/agent-1/guest-operations/exact%2Fconnection",
  );
  assert.deepEqual(calls[0]?.options?.body, payload);
  await assert.rejects(
    api.saveOperations("agent-1", false, "id", {
      ...selection,
      rules: [{ method: "GET", path_template: "https://evil.invalid" }],
    }),
  );
  assert.equal(calls.length, 1);
});

test("expired/revoked invitation and stale revision errors propagate without legacy fallback", async () => {
  const calls: string[] = [];
  const api = createCollaborationApi(async (path) => {
    calls.push(path);
    throw new Error("409: membership changed or invitation expired");
  });
  await assert.rejects(api.accept(TOKEN), /invitation expired/);
  await assert.rejects(
    api.change(ID, 7, { action: "remove", person_id: USER }),
    /membership changed/,
  );
  assert.deepEqual(calls, [
    "/assistant/nyxagent/group-invites/accept",
    `/assistant/nyxagent/groups/${ID}/membership`,
  ]);
});

test("mutation authority guards reach the authenticated dispatch boundary without entering the JSON body", async () => {
  let active = true;
  const guard = () => {
    if (!active) throw new Error("backgrounded");
  };
  let writes = 0;
  const api = createCollaborationApi(async (_path, options) => {
    assert.equal(options?.authorityGuard, guard);
    assert.equal(
      Object.hasOwn(options?.body as object, "authorityGuard"),
      false,
    );
    await Promise.resolve();
    options?.authorityGuard?.();
    writes++;
    return {};
  });
  const pending = api.decideConsent("consent", "once", {
    authorityGuard: guard,
  });
  active = false;
  await assert.rejects(pending, /backgrounded/);
  assert.equal(writes, 0);
});
