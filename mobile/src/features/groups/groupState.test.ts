import assert from "node:assert/strict";
import { test } from "node:test";
import {
  groupSchema,
  groupMessageSchema,
  groupPendingActionSchema,
  type GroupMessagesPage,
} from "./groupContracts";
import {
  NO_RECIPIENT_EDITS,
  withDraft,
  withAdded,
  withRemoved,
  resolveRecipients,
  mentionedIds,
  mergeGroupMessages,
  mergeGroupPage,
  groupCanManage,
  groupCanLeave,
  eligibleGroupAgents,
  eligibleGroupPeople,
  groupPendingReview,
  groupCanDecide,
} from "./groupState";
import { agentSchema } from "../agents/agentContracts";

const NOW = "2026-10-15T08:00:00Z";
const ID = "nyxg-0123456789abcdef0123456789abcdef";
const USER = "1785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const OTHER = "2785c70a-9a5b-41f8-b73e-6ff523cdd98e";
const ACK = "e8aa62de-1c12-4ba0-b90b-92b99290d35d";
const members = [
  { id: "a", name: "ana" },
  { id: "b", name: "research_bot" },
  { id: "c", name: "gone", destroyed: true },
];
const group = groupSchema.parse({
  id: ID,
  name: "Research",
  members: members.map((m) => ({ ...m, kind: "specialist" })),
  lead_agent_id: "a",
  created_at: NOW,
});
const message = (seq: number, text = `Message ${seq}`) =>
  groupMessageSchema.parse({
    id: ACK,
    seq,
    role: "user",
    text,
    created_at: NOW,
  });
const page = (
  seqs: number[],
  before: number | null,
  name = "Research",
): GroupMessagesPage => ({
  group: { ...group, name },
  messages: seqs.map((seq) => message(seq)),
  before_seq: before,
  pending_actions: [],
});

test("mentions follow server handle boundaries, case and destroyed-member rules", () => {
  assert.deepEqual(
    mentionedIds(
      "mail@ana @ANA, (@research_BOT) @gone @ana .@ana 汉@ana",
      members,
    ),
    ["a", "b"],
  );
  assert.deepEqual(mentionedIds("@Researcher", members), []);
});
test("unedited routing uses mentions then sticky defaults and filters destroyed recipients", () => {
  assert.deepEqual(
    resolveRecipients(NO_RECIPIENT_EDITS, ["a", "a", "c"], members),
    { ids: ["a"], edited: false },
  );
  assert.deepEqual(
    resolveRecipients(
      withDraft(NO_RECIPIENT_EDITS, "ask @research_bot", members),
      ["a"],
      members,
    ),
    { ids: ["b"], edited: false },
  );
});
test("manual pills are explicit, can remove every recipient, and a fresh remention restores one", () => {
  let selection = withDraft(NO_RECIPIENT_EDITS, "@ana", members);
  selection = withAdded(selection, "b");
  selection = withAdded(selection, "b");
  assert.deepEqual(resolveRecipients(selection, ["a"], members), {
    ids: ["a", "b"],
    edited: true,
  });
  selection = withRemoved(withRemoved(selection, "a"), "b");
  assert.deepEqual(resolveRecipients(selection, ["a"], members), {
    ids: [],
    edited: true,
  });
  selection = withDraft(selection, "", members);
  selection = withDraft(selection, "@ana", members);
  assert.deepEqual(resolveRecipients(selection, ["a"], members), {
    ids: ["a"],
    edited: true,
  });
  assert.deepEqual(
    resolveRecipients(withRemoved(NO_RECIPIENT_EDITS, "c"), ["a"], members),
    { ids: ["a"], edited: false },
  );
});
test("message merging replaces overlaps and keeps ascending sequence order", () => {
  assert.deepEqual(
    mergeGroupMessages(
      [message(2), message(1)],
      [message(2, "updated"), message(3)],
    ).map((m) => [m.seq, m.text]),
    [
      [1, "Message 1"],
      [2, "updated"],
      [3, "Message 3"],
    ],
  );
});
test("old-page loads preserve current group and pending metadata and advance oldest cursor", () => {
  const current = page([5, 6], 5, "Live name");
  const merged = mergeGroupPage(
    current,
    page([3, 4, 5], 3, "Old name"),
    "older",
  );
  assert.equal(merged.group.name, "Live name");
  assert.equal(merged.before_seq, 3);
  assert.deepEqual(
    merged.messages.map((m) => m.seq),
    [3, 4, 5, 6],
  );
});
test("refresh replaces newest range while retaining loaded history and oldest cursor", () => {
  const current = page([1, 2, 3, 4, 5, 6], 1, "Old name");
  const merged = mergeGroupPage(current, page([4, 6, 7], 4, "Live name"));
  assert.equal(merged.before_seq, 1);
  assert.equal(merged.group.name, "Live name");
  assert.deepEqual(
    merged.messages.map((m) => m.seq),
    [1, 2, 3, 4, 6, 7],
  );
  assert.throws(
    () =>
      mergeGroupPage(current, {
        ...current,
        group: { ...group, id: "nyxg-1123456789abcdef0123456789abcdef" },
      }),
    /different groups/,
  );
});
test("group management derives from participating group role with legacy personal support", () => {
  assert.equal(groupCanManage(group), true);
  assert.equal(groupCanLeave(group), false);
  const org = {
    ...group,
    owner: { type: "org" as const, id: USER, name: "Org" },
  };
  assert.equal(groupCanManage(org), false);
  assert.equal(groupCanManage({ ...org, your_role: "participant" }), false);
  assert.equal(groupCanManage({ ...org, your_role: "creator" }), true);
  assert.equal(groupCanManage({ ...org, your_role: "admin" }), true);
  assert.equal(groupCanLeave({ ...org, your_role: "participant" }), true);
});
test("personal groups allow usable shared org agents; org groups restrict exact owner", () => {
  const base = {
    name: "researcher",
    kind: "specialist",
    description: "Research",
    status: "idle",
  };
  const agents = [
    agentSchema.parse({ ...base, id: "personal", owner_kind: "person" }),
    agentSchema.parse({
      ...base,
      id: "shared",
      owner_kind: "org",
      owner_id: USER,
      can_use: true,
    }),
    agentSchema.parse({
      ...base,
      id: "blocked",
      owner_kind: "org",
      owner_id: USER,
      can_use: false,
    }),
    agentSchema.parse({ ...base, id: "destroyed", status: "destroyed" }),
  ];
  assert.deepEqual(
    eligibleGroupAgents(agents).map((a) => a.id),
    ["personal", "shared"],
  );
  assert.deepEqual(
    eligibleGroupAgents(agents, USER).map((a) => a.id),
    ["shared"],
  );
  assert.deepEqual(
    eligibleGroupPeople([
      { membership_id: ACK, user_id: USER, role: "member", revoked_at: null },
      { membership_id: ACK, user_id: OTHER, role: "viewer", revoked_at: null },
      { membership_id: ACK, user_id: OTHER, role: "admin", revoked_at: NOW },
    ]).map((p) => p.user_id),
    [USER],
  );
});
test("authored reviews use owned conversation and acknowledgement IDs, ignoring review URL", () => {
  const action = groupPendingActionSchema.parse({
    conversation_id: "nyxa-0123456789abcdef0123456789abcdef",
    acknowledgement_id: ACK,
    authored_skill: true,
    summary: "Draft",
    review_url: "https://untrusted.invalid/other",
    triggering_person: { id: USER, display_name: "Owner" },
    expires_at: "2026-10-16T00:00:00Z",
  });
  const now = Date.parse(NOW);
  assert.deepEqual(groupPendingReview(action, USER, true, now), {
    kind: "assistant",
    conversationId: action.conversation_id,
    acknowledgementId: ACK,
  });
  assert.equal(groupPendingReview(action, OTHER, true, now), undefined);
  assert.equal(
    groupPendingReview(action, USER, true, Date.parse("2026-10-17T00:00:00Z")),
    undefined,
  );
  assert.equal(
    groupCanDecide({ ...action, can_decide: true }, USER, true, now),
    false,
  );
});
test("organization decisions require both server authority and triggering-person ownership", () => {
  const action = groupPendingActionSchema.parse({
    conversation_id: "nyxa-0123456789abcdef0123456789abcdef",
    approval_request_id: ACK,
    summary: "Connect",
    can_decide: true,
    confirm_phrase: "yes 4821",
    triggering_person: { id: USER, display_name: "Owner" },
    expires_at: "2026-10-16T00:00:00Z",
  });
  const now = Date.parse(NOW);
  assert.equal(groupCanDecide(action, USER, true, now), true);
  assert.equal(groupCanDecide(action, OTHER, true, now), false);
  assert.equal(
    groupCanDecide({ ...action, can_decide: false }, USER, true, now),
    false,
  );
  assert.equal(groupPendingReview(action, USER, true, now), undefined);
});
