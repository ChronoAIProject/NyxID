import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import {
  authoredSkillActionRequest,
  authoredSkillPreviewSchema,
  authoredReviewMatches,
  interruptedPublicationText,
  publicationFailureText,
  skillDraftDiagnostics,
  skillDraftDiagnosticText,
  supportedAuthoredSkillActions,
  type AuthoredSkillAction,
} from "./authoredSkillReviewState";

const reference = {
  agent_id: "agent-a",
  proposal_id: "proposal-a",
  revision: 4,
  skills_revision: 7,
};
const data = authoredSkillPreviewSchema.parse({
  id: "proposal-a",
  agent_id: "agent-a",
  agent_name: "Researcher",
  revision: 4,
  skills_revision: 7,
  current_skills_revision: 9,
  status: "pending",
  state: "active",
  actions: ["deny", "publish", "renew", "discard", "reprepare"],
  name: "research-digest",
  version: "1.2.0",
  files: [{ path: "SKILL.md", content: "Reviewed plain-text workflow" }],
});

test("publish and deny use the exact acknowledgement decision contract", () => {
  assert.deepEqual(
    authoredSkillActionRequest(reference, data, "ack-a", "publish"),
    { kind: "decision", choice: "allow" },
  );
  assert.deepEqual(
    authoredSkillActionRequest(reference, data, "ack-a", "deny"),
    { kind: "decision", choice: "deny" },
  );
});

test("an unadvertised retry cannot turn an uncertain publication into a new write", () => {
  const uncertain = {
    ...data,
    status: "publishing",
    lease_live: false,
    actions: ["check"],
  };
  assert.deepEqual(
    authoredSkillActionRequest(reference, uncertain, "ack-a", "check"),
    { kind: "decision", choice: "allow" },
  );
  assert.throws(
    () => authoredSkillActionRequest(reference, uncertain, "ack-a", "retry"),
    /no longer available/,
  );
  assert.match(
    interruptedPublicationText(uncertain.actions),
    /never writes again/,
  );
});

test("only an explicit nothing-sent recovery is described as retryable", () => {
  const retryable = {
    ...data,
    status: "publishing",
    actions: ["retry", "discard"],
  };
  assert.deepEqual(
    authoredSkillActionRequest(reference, retryable, "ack-a", "retry"),
    { kind: "decision", choice: "allow" },
  );
  assert.match(
    interruptedPublicationText(retryable.actions),
    /before anything was sent/,
  );
});

test("renew carries reviewed draft revision, current skills revision and exact old card", () => {
  assert.deepEqual(
    authoredSkillActionRequest(reference, data, "ack-a", "renew"),
    {
      kind: "post",
      action: "approve",
      body: { revision: 4, agent_skills_revision: 9, renewal_of: "ack-a" },
    },
  );
});

test("discard is revision fenced and rebuilding targets the exact card", () => {
  assert.deepEqual(
    authoredSkillActionRequest(reference, data, "ack-a", "discard"),
    {
      kind: "post",
      action: "reject",
      body: { revision: 4, reason: "rejected" },
    },
  );
  assert.deepEqual(
    authoredSkillActionRequest(reference, data, "ack-a", "reprepare"),
    {
      kind: "post",
      action: "reprepare",
      body: { acknowledgement_id: "ack-a" },
    },
  );
});

test("legacy cards without complete review metadata fail closed", () => {
  assert.equal(
    authoredReviewMatches(
      { agent_id: "agent-a", proposal_id: "proposal-a" },
      data,
    ),
    false,
  );
  assert.throws(
    () =>
      authoredSkillActionRequest(
        { ...reference, revision: undefined },
        data,
        "ack-a",
        "publish",
      ),
    /does not match/,
  );
  assert.throws(
    () =>
      authoredSkillActionRequest(
        { ...reference, skills_revision: undefined },
        data,
        "ack-a",
        "deny",
      ),
    /does not match/,
  );
});

test("another proposal, agent or active draft revision cannot use this confirmation", () => {
  for (const changed of [
    { ...data, id: "another" },
    { ...data, agent_id: "another" },
    { ...data, revision: 5 },
  ]) {
    assert.throws(
      () => authoredSkillActionRequest(reference, changed, "ack-a", "publish"),
      /does not match/,
    );
  }
});

test("a renewed card may bind a newer skills revision than the previous publication snapshot", () => {
  const renewed = { ...reference, skills_revision: 9 };
  // GET is card-bound. The backend advertises publish for the renewed card,
  // while publication.skills_revision remains 7 until the attempt is claimed.
  assert.deepEqual(
    authoredSkillActionRequest(renewed, data, "ack-renewed", "publish"),
    { kind: "decision", choice: "allow" },
  );
});

test("a stale rebuilt card can only show the updated draft when the server permits it", () => {
  const stale = {
    ...data,
    state: "changed",
    revision: 5,
    actions: ["show_updated_draft"],
  };
  assert.deepEqual(
    authoredSkillActionRequest(reference, stale, "ack-a", "show_updated_draft"),
    {
      kind: "post",
      action: "reprepare",
      body: { acknowledgement_id: "ack-a" },
    },
  );
  assert.throws(
    () => authoredSkillActionRequest(reference, stale, "ack-a", "publish"),
    /no longer available/,
  );
});

test("unknown future review states and terminal states never offer writes", () => {
  for (const state of [
    "future_state",
    "pinned",
    "discarded",
    "invalidated",
    "dismissed",
  ]) {
    const unknown = {
      ...data,
      state,
      actions: ["publish", "deny", "show_updated_draft"],
    };
    assert.deepEqual(supportedAuthoredSkillActions(unknown), []);
    assert.throws(() =>
      authoredSkillActionRequest(reference, unknown, "ack-a", "publish"),
    );
  }
});

test("a changed card never advertises publication even if a server mistakenly includes it", () => {
  const changed = {
    ...data,
    state: "changed",
    revision: 5,
    actions: ["publish", "renew", "show_updated_draft"],
  };
  assert.deepEqual(supportedAuthoredSkillActions(changed), [
    "show_updated_draft",
  ]);
  assert.throws(
    () => authoredSkillActionRequest(reference, changed, "ack-a", "publish"),
    /no longer available/,
  );
});

test("unknown future actions never fall through into a different server operation", () => {
  assert.throws(
    () =>
      authoredSkillActionRequest(
        reference,
        { ...data, actions: ["future_operation"] },
        "ack-a",
        "future_operation" as AuthoredSkillAction,
      ),
    /no longer available/,
  );
});

test("publication errors distinguish verified-but-unattached from an uncertain write", () => {
  assert.match(
    publicationFailureText("base_changed", "published_unpinned"),
    /verified.*did not attach/,
  );
  assert.match(
    publicationFailureText("publish_uncertain", "publishing"),
    /nothing else will be written/,
  );
  assert.equal(
    publicationFailureText("untrusted_provider_error", "failed"),
    "Publication could not finish. Check the current state.",
  );
});

test("structured diagnostics keep fields, lines and bounds without echoing private content", () => {
  const diagnostics = skillDraftDiagnostics({
    details: [
      {
        field: "skill_md",
        rule: "too_large",
        line: 12,
        limit: 1000,
        actual: 1200,
        unit: "bytes",
      },
      {
        field: "files[1].content",
        rule: "private_material",
        line: 4,
        unsafe_content: "secret-value",
      },
      { unexpected: "ignored" },
    ],
  });
  assert.equal(diagnostics.length, 2);
  assert.match(
    skillDraftDiagnosticText(diagnostics[0]!),
    /Skill draft.*line 12.*1,000 bytes.*1,200/,
  );
  assert.match(
    skillDraftDiagnosticText(diagnostics[1]!),
    /File 2 content.*private material.*line 4/,
  );
  assert.doesNotMatch(JSON.stringify(diagnostics), /secret-value/);
});

test("single and Web-compatible diagnostics remain readable", () => {
  assert.deepEqual(
    skillDraftDiagnostics({ details: { field: "name", rule: "required" } }),
    [{ field: "name", rule: "required" }],
  );
  assert.deepEqual(
    skillDraftDiagnostics({
      errorResponse: {
        details: [{ field: "files[0].path", rule: "unsafe_path" }],
      },
    }),
    [{ field: "files[0].path", rule: "unsafe_path" }],
  );
  assert.deepEqual(skillDraftDiagnostics(new Error("No diagnostics")), []);
});

test("ApiError preserves structured diagnostics through the native draft adapter without echoing private content", () => {
  const error = new ApiError({
    errorKey: "validation_error",
    errorCode: 1000,
    statusCode: 400,
    message: "The skill draft contains private material.",
    details: [
      {
        field: "skill_md",
        rule: "private_material",
        line: 8,
        private_input: "private-token-value-do-not-display",
      },
    ],
  });
  assert.equal(error.statusCode, 400);
  const diagnostics = skillDraftDiagnostics(error);
  assert.deepEqual(diagnostics, [
    { field: "skill_md", rule: "private_material", line: 8 },
  ]);
  const output = diagnostics.map(skillDraftDiagnosticText).join("\n");
  assert.match(output, /Skill draft.*private material.*line 8/);
  assert.doesNotMatch(output, /private-token-value-do-not-display/);
  assert.doesNotMatch(
    JSON.stringify(diagnostics),
    /private_input|private-token-value-do-not-display/,
  );
});
