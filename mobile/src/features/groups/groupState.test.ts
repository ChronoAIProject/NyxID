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
  ambiguousGroupMentions,
  groupMentionBindings,
  unavailableGroupRecipients,
  mergeGroupMessages,
  mergeGroupPage,
  groupCanManage,
  groupCanLeave,
  eligibleGroupAgents,
  eligibleGroupPeople,
  groupPendingReview,
  groupCanDecide,
  groupWorkingLabel,
} from "./groupState";

test("crew working label matches web singular, pair and overflow wording", () => {
  assert.equal(groupWorkingLabel([]), "");
  assert.equal(groupWorkingLabel(["NyxBot"]), "NyxBot is working…");
  assert.equal(
    groupWorkingLabel(["NyxBot", "Researcher"]),
    "NyxBot and Researcher are working…",
  );
  assert.equal(
    groupWorkingLabel(["NyxBot", "Researcher", "Writer", "Tester"]),
    "NyxBot, Researcher and 2 more are working…",
  );
});
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

test("shared namesakes never resolve by bare handle and unavailable selections cannot fall back", () => {
  const roster = [
    { id: USER, name: "research" },
    { id: OTHER, name: "research", unavailable: true },
    { id: "third", name: "write" },
  ];
  assert.deepEqual(ambiguousGroupMentions("@research please help", roster), [
    "research",
  ]);
  assert.deepEqual(mentionedIds("@research", roster), []);
  assert.deepEqual(mentionedIds(`@${USER}`, roster), [USER]);
  assert.deepEqual(mentionedIds(`@${OTHER}`, roster), []);
  const selection = withAdded(NO_RECIPIENT_EDITS, OTHER);
  assert.deepEqual(resolveRecipients(selection, ["third"], roster).ids, []);
});

test("revoked or destroyed mentioned agents retain their exact target and block fallback", () => {
  for (const status of [{ unavailable: true }, { destroyed: true }]) {
    for (const mention of ["research", USER]) {
      const live = [
        { id: USER, name: "research" },
        { id: OTHER, name: "default" },
      ];
      const text = `@${mention} confidential question`;
      const draft = withDraft(NO_RECIPIENT_EDITS, text, live);
      const changed = [{ ...live[0]!, ...status }, live[1]!];
      const retained = withDraft(draft, text, changed);
      assert.deepEqual(retained.mentioned, [USER]);
      assert.deepEqual(resolveRecipients(retained, [OTHER], changed).ids, []);
      assert.deepEqual(unavailableGroupRecipients(retained, changed), [USER]);
      const replacement = withAdded(withRemoved(retained, USER), OTHER);
      assert.deepEqual(unavailableGroupRecipients(replacement, changed), []);
      assert.deepEqual(resolveRecipients(replacement, [OTHER], changed).ids, [
        OTHER,
      ]);
    }
  }
});

test("an exact UUID absent from the current roster cannot fall back to a default", () => {
  const roster = [{ id: OTHER, name: "default" }];
  const selection = withDraft(NO_RECIPIENT_EDITS, `@${USER} question`, roster);
  assert.deepEqual(resolveRecipients(selection, [OTHER], roster).ids, []);
  assert.deepEqual(unavailableGroupRecipients(selection, roster), [USER]);
});

test("resolved handles retain exact identity across rename and handle reuse until edited away", () => {
  const original = [{ id: USER, name: "research" }];
  const text = "@research confidential question";
  const selection = withDraft(NO_RECIPIENT_EDITS, text, original);
  const bindings = groupMentionBindings(text, original);
  const changed = [
    { id: USER, name: "analyst" },
    { id: OTHER, name: "research" },
  ];
  const retained = withDraft(selection, text, changed, bindings);
  assert.deepEqual(retained.mentioned, [USER]);
  assert.deepEqual(resolveRecipients(retained, [OTHER], changed).ids, [USER]);
  const removed = changed.slice(1);
  assert.deepEqual(unavailableGroupRecipients(retained, removed), [USER]);
  assert.deepEqual(resolveRecipients(retained, [OTHER], removed).ids, []);
  const cleared = withDraft(selection, "A new question", changed, bindings);
  const clearedBindings = groupMentionBindings(
    "A new question",
    changed,
    bindings,
  );
  assert.deepEqual(clearedBindings, []);
  assert.deepEqual(
    withDraft(cleared, text, changed, clearedBindings).mentioned,
    [OTHER],
  );
});

test("unknown handles remain ordinary text and bound names cannot override an exact UUID", () => {
  const roster = [{ id: OTHER, name: "default" }];
  const unknown = withDraft(
    NO_RECIPIENT_EDITS,
    "Ask @outside about this",
    roster,
  );
  assert.deepEqual(unknown.mentioned, []);
  assert.deepEqual(unavailableGroupRecipients(unknown, roster), []);
  assert.deepEqual(resolveRecipients(unknown, [OTHER], roster).ids, [OTHER]);
  assert.deepEqual(
    groupMentionBindings(`@${USER}`, roster, [{ id: OTHER, name: USER }]),
    [{ id: USER, name: USER }],
  );
});

test("collaboration management is decided by the explicit policy, never personal legacy ownership", () => {
  const collaboration = {
    version: 1 as const,
    creator_user_id: USER,
    membership_revision: 1,
    history_disclosure: "History",
    can_manage: false,
    your_request_ids: [],
    transfer_to_user_id: null,
    guest_access: [],
    proposals: [],
    invites: [],
  };
  assert.equal(groupCanManage({ ...group, collaboration }), false);
  assert.equal(groupCanLeave({ ...group, collaboration }), true);
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
    /different crews/,
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
