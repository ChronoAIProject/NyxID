import assert from "node:assert/strict";
import test from "node:test";
import {
  agentSkillsSchema,
  assertSkillRevision,
  buildSkillSelection,
  skillPreviewSchema,
  type AgentSkill,
  type AgentSkills,
} from "./pluginSkillsTypes";

const pin = (id: string, extra: Record<string, unknown> = {}): AgentSkill => ({
  source: "ornn",
  skill_id: id,
  name: `skill-${id}`,
  version: "1.0.0",
  sha256: `hash-${id}`,
  dependencies: [],
  disabled: false,
  ...extra,
});
const current = (
  skills: AgentSkill[] = [pin("first"), pin("second")],
): AgentSkills => ({
  agent_id: "agent-a",
  revision: 7,
  skills,
  metadata: {},
});
const preview = skillPreviewSchema.parse({
  reference: pin("third", {
    version: "2.3.4",
    sha256: "hash-third-v2",
    dependencies: [pin("dependency")],
  }),
  description: "A verified exact package",
  size_bytes: 2048,
});
const selected = {
  version: "2.3.4",
  sha256: "hash-third-v2",
  deprecated: false,
};

test("attaching sends the selected exact version and preserves unrelated pins", () => {
  const baseline = current();
  const result = buildSkillSelection(baseline, {
    type: "attach",
    skillId: "third",
    selected,
    preview,
  });
  assert.equal(result.expected_revision, 7);
  assert.deepEqual(result.skills.slice(0, 2), baseline.skills);
  assert.equal(result.skills[2]?.version, "2.3.4");
  assert.deepEqual(
    result.skills[2]?.dependencies,
    preview.reference.dependencies,
  );
  assert.equal(baseline.skills.length, 2);
});

test("a different package digest cannot be attached under the selected version", () => {
  assert.throws(
    () =>
      buildSkillSelection(current(), {
        type: "attach",
        skillId: "third",
        selected: { ...selected, sha256: "changed-content" },
        preview,
      }),
    /does not match/,
  );
});

test("a preview for another skill or literal version cannot be attached", () => {
  assert.throws(
    () =>
      buildSkillSelection(current(), {
        type: "attach",
        skillId: "another",
        selected,
        preview,
      }),
    /does not match/,
  );
  assert.throws(
    () =>
      buildSkillSelection(current(), {
        type: "attach",
        skillId: "third",
        selected: { ...selected, version: "latest" },
        preview,
      }),
    /does not match/,
  );
});

test("a disabled pin cannot be re-pinned or silently enabled", () => {
  const baseline = agentSkillsSchema.parse(
    current([
      pin("third", {
        disabled: true,
        activation: { mode: "on_demand" },
        publisher_id: "existing-publisher",
      }),
    ]),
  );
  assert.throws(
    () =>
      buildSkillSelection(baseline, {
        type: "attach",
        skillId: "third",
        selected,
        preview,
      }),
    /Enable this skill before updating/,
  );
  assert.equal(baseline.skills[0]?.disabled, true);
  assert.equal(baseline.skills[0]?.version, "1.0.0");
});

test("re-pinning an enabled skill preserves additive activation settings", () => {
  const baseline = agentSkillsSchema.parse(
    current([
      pin("third", {
        activation: { mode: "on_demand" },
        publisher_id: "existing-publisher",
      }),
    ]),
  );
  const result = buildSkillSelection(baseline, {
    type: "attach",
    skillId: "third",
    selected,
    preview,
  });
  assert.equal(result.skills[0]?.disabled, false);
  assert.deepEqual(result.skills[0]?.activation, { mode: "on_demand" });
  assert.equal(result.skills[0]?.publisher_id, "existing-publisher");
  assert.equal(result.skills[0]?.version, "2.3.4");
});

test("selection round-trips unknown fields and dependency metadata on sibling pins", () => {
  const baseline = agentSkillsSchema.parse(
    current([
      pin("first", {
        activation_mode: "always",
        dependencies: [pin("dep", { publisher_id: "publisher-a" })],
      }),
      pin("second"),
    ]),
  );
  const result = buildSkillSelection(baseline, {
    type: "detach",
    skillId: "second",
  });
  assert.equal(result.skills[0]?.activation_mode, "always");
  assert.equal(result.skills[0]?.dependencies[0]?.publisher_id, "publisher-a");
  assert.equal(result.skills[0]?.version, "1.0.0");
});

test("toggle changes only the chosen pin's disabled state", () => {
  const baseline = current();
  const result = buildSkillSelection(baseline, {
    type: "toggle",
    skillId: "first",
    disabled: true,
  });
  assert.deepEqual(result.skills[0], { ...baseline.skills[0], disabled: true });
  assert.deepEqual(result.skills[1], baseline.skills[1]);
  assert.equal(baseline.skills[0]?.disabled, false);
});

test("16 pins blocks a new attachment but permits replacement", () => {
  const full = current(Array.from({ length: 16 }, (_, i) => pin(String(i))));
  assert.throws(
    () =>
      buildSkillSelection(full, {
        type: "attach",
        skillId: "third",
        selected,
        preview,
      }),
    /16 skills/,
  );
  const replacement = {
    ...preview,
    reference: { ...preview.reference, skill_id: "0" },
  };
  assert.equal(
    buildSkillSelection(full, {
      type: "attach",
      skillId: "0",
      selected,
      preview: replacement,
    }).skills.length,
    16,
  );
});

test("stale detach and toggle fail instead of claiming a successful change", () => {
  assert.throws(
    () =>
      buildSkillSelection(current(), { type: "detach", skillId: "missing" }),
    /no longer attached/,
  );
  assert.throws(
    () =>
      buildSkillSelection(current(), {
        type: "toggle",
        skillId: "missing",
        disabled: true,
      }),
    /no longer attached/,
  );
});

test("a concurrent edit or another agent's response stops the write", () => {
  assert.throws(
    () =>
      assertSkillRevision("agent-a", current(), { ...current(), revision: 8 }),
    /changed elsewhere/,
  );
  assert.throws(
    () =>
      assertSkillRevision("agent-a", current(), {
        ...current(),
        agent_id: "agent-b",
      }),
    /changed elsewhere/,
  );
  assert.doesNotThrow(() =>
    assertSkillRevision("agent-a", current(), current()),
  );
});
