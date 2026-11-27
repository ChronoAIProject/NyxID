import assert from "node:assert/strict";
import test from "node:test";
import { LANGUAGES } from "../../i18n/language";
import { translate } from "../../i18n/translate";
import { agentSchema } from "./agentContracts";
import { rosterActivityLabel, rosterAvatarMood } from "./agentRosterMood";
import { ROSTER_ACTIVITY_STATES } from "../../../../shared/agent-avatars/activity";

const agent = {
  id: "research",
  name: "researcher",
  kind: "specialist",
  description: "",
  status: "running",
  pending_acknowledgements: 1,
};

test("mobile roster accepts omitted activity and drops unknown or malformed items", () => {
  const legacy = agentSchema.parse(agent);
  assert.equal(legacy.activity, undefined);
  assert.equal(rosterAvatarMood(legacy), "approval");
  const current = agentSchema.parse({
    ...agent,
    activity: [
      { conversation_id: "first", state: "waiting" },
      { conversation_id: "second", state: "drafting" },
      { conversation_id: "future", state: "future" },
      { state: "approval" },
      null,
    ],
  });
  assert.deepEqual(current.activity, [
    { conversation_id: "first", state: "waiting" },
    { conversation_id: "second", state: "drafting" },
  ]);
  assert.equal(rosterAvatarMood(current), "drafting");
  assert.equal(rosterAvatarMood({ ...current, activity: [] }), "idle");
  assert.equal(rosterAvatarMood({ ...current, status: "destroyed" }), "idle");
});

test("all shipped mobile languages localize the accessible roster activity", () => {
  for (const { code } of LANGUAGES) {
    for (const state of ROSTER_ACTIVITY_STATES) {
      const phrase = rosterActivityLabel(state);
      const status = translate(code, phrase);
      const name = translate(code, "Manage {{name}}, {{status}}", {
        name: "Research",
        status,
      });
      assert.ok(name.includes("Research") && name.includes(status));
      if (code !== "en") {
        assert.notEqual(status, phrase);
        assert.doesNotMatch(name, /Manage|\{\{/u);
      }
    }
  }
});
