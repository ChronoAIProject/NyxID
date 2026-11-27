import assert from "node:assert/strict";
import test from "node:test";
import { translate } from "../../i18n/translate";
import simplified from "../../i18n/locales/zh-CN";
import traditional from "../../i18n/locales/zh-TW";
import {
  AVATAR_FIELDS,
  AVATAR_MOODS,
  AVATAR_TEMPLATE,
  avatarChoiceLabel,
  avatarChoices,
  avatarMoodLabel,
  canEditAvatar,
  defaultAvatar,
} from "../../lib/avatars/agentAvatars";
import { agentSchema, type AssistantAgent } from "./agentContracts";
import {
  AVATAR_EDITOR_COPY,
  avatarChangedElsewhere,
  avatarEditorState,
} from "./avatarEditorModel";
import { groupMemberSchema } from "../groups/groupContracts";
import { notificationActorSchema } from "../notifications/notificationContracts";
import { machineActivitySchema } from "../machines/machineContracts";
import { conversationSchema } from "../assistant/assistantTypes";

const ID = "6f1c2a8e-3b4d-4c5e-8f90-123456789abc";
const chosen = {
  template: AVATAR_TEMPLATE,
  choices: { head: "round", eyes: "visor", palette: "core-ocean" },
};
const agent = (extra: Record<string, unknown> = {}): AssistantAgent =>
  agentSchema.parse({
    id: ID,
    kind: "specialist",
    name: "researcher",
    description: "",
    status: "idle",
    can_maintain: true,
    ...extra,
  });

test("agents carry their avatar; malformed or absent means the default", () => {
  assert.deepEqual(agent({ avatar: chosen }).avatar, chosen);
  assert.equal(agent({ avatar: { template: 1 } }).avatar, null);
  assert.equal(agent().avatar, undefined);
  // A newer release's ids still parse; drawing falls back to the default.
  assert.equal(
    agent({
      avatar: { ...chosen, choices: { ...chosen.choices, eyes: "future" } },
    }).avatar?.choices.eyes,
    "future",
  );
});

test("every mobile surface that shows an agent parses its avatar", () => {
  const member = groupMemberSchema.parse({
    id: ID,
    name: "r",
    kind: "specialist",
    avatar: chosen,
  });
  assert.deepEqual(member.avatar, chosen);
  assert.deepEqual(
    notificationActorSchema.parse({ type: "agent", name: "r", avatar: chosen })
      .avatar,
    chosen,
  );
  assert.equal(
    notificationActorSchema.parse({ type: "agent", name: "r", avatar: "bad" })
      .avatar,
    null,
  );
  const machines = machineActivitySchema.shape.agents.parse([
    { id: ID, name: "r", kind: "specialist", avatar: chosen },
  ]);
  assert.deepEqual(machines[0]!.avatar, chosen);
  const agentField = conversationSchema.shape.agent.parse({
    id: ID,
    name: "r",
    kind: "specialist",
    avatar: chosen,
  });
  assert.deepEqual(agentField?.avatar, chosen);
});

test("saving: a new choice sets it, back to default clears the stored one", () => {
  const fallback = defaultAvatar(ID, "specialist");
  const fresh = agent();
  assert.equal(avatarEditorState(fresh, fallback).dirty, false);
  const set = avatarEditorState(fresh, chosen);
  assert.equal(set.dirty, true);
  assert.deepEqual(set.patch, { avatar: chosen });
  const stored = agent({ avatar: chosen });
  const reset = avatarEditorState(stored, fallback);
  assert.equal(reset.dirty, true);
  assert.equal(reset.atDefault, true);
  assert.deepEqual(reset.patch, { avatar: null });
});

test("a change made elsewhere since opening is refused", () => {
  assert.equal(avatarChangedElsewhere(agent(), agent()), false);
  assert.equal(
    avatarChangedElsewhere(agent(), agent({ avatar: chosen })),
    true,
  );
  assert.equal(
    avatarChangedElsewhere(agent({ avatar: chosen }), agent()),
    true,
  );
  assert.equal(
    avatarChangedElsewhere(
      agent({ avatar: chosen }),
      agent({ avatar: { ...chosen } }),
    ),
    false,
  );
  assert.equal(
    avatarChangedElsewhere(
      agent({ avatar: chosen }),
      agent({
        avatar: { ...chosen, choices: { ...chosen.choices, eyes: "happy" } },
      }),
    ),
    true,
  );
});

test("maintainers and shared managers may restyle; viewers and users may not", () => {
  assert.equal(canEditAvatar(agent()), true);
  assert.equal(canEditAvatar(agent({ can_maintain: false })), false);
  assert.equal(
    canEditAvatar(agent({ share_role: "manager", can_maintain: false })),
    true,
  );
  assert.equal(canEditAvatar(agent({ share_role: "user" })), false);
  assert.equal(canEditAvatar(agent({ status: "destroyed" })), false);
});

test("every avatar label and editor phrase is translated in both catalogs", () => {
  const phrases = [
    ...AVATAR_FIELDS.flatMap((field) =>
      avatarChoices(field).map((id) => avatarChoiceLabel(field, id)),
    ),
    ...AVATAR_MOODS.map(avatarMoodLabel),
    ...AVATAR_EDITOR_COPY,
  ];
  assert.ok(phrases.length > 60);
  for (const [language, catalog] of [
    ["zh-CN", simplified],
    ["zh-TW", traditional],
  ] as const) {
    for (const phrase of phrases) {
      assert.ok(phrase in catalog, `${language} is missing "${phrase}"`);
      if (phrase === "Nyx") {
        // The shared shape/brand name stays Nyx in every language.
        assert.equal(translate(language, phrase), "Nyx");
      } else {
        assert.notEqual(
          translate(language, phrase),
          phrase,
          `${language}: "${phrase}"`,
        );
      }
    }
  }
});
