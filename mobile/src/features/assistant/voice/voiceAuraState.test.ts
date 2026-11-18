import assert from "node:assert/strict";
import test from "node:test";
import {
  OUTPUT_SPEAKING_RELEASE_MS,
  levelTransitionMs,
  nextOutputActivity,
  voiceAuraPhase,
  voiceAuraStatus,
  voiceAuraTargets,
  type VoiceAuraInput,
} from "./voiceAuraState";

const live: VoiceAuraInput = {
  starting: false,
  connected: true,
  inputMode: "automatic",
  holding: false,
  microphoneBlocked: false,
  muted: false,
  outputActive: false,
  captions: [],
  tasks: [],
};

test("the aura phase follows connection, speech, turns and tasks", () => {
  assert.equal(voiceAuraPhase({ ...live, connected: false }), "resting");
  assert.equal(voiceAuraPhase({ ...live, starting: true }), "connecting");
  assert.equal(voiceAuraPhase(live), "listening");
  assert.equal(voiceAuraPhase({ ...live, outputActive: true }), "speaking");
  assert.equal(
    voiceAuraPhase({
      ...live,
      captions: [{ speaker: "assistant", complete: false }],
    }),
    "speaking",
  );
  assert.equal(
    voiceAuraPhase({ ...live, captions: [{ speaker: "user", complete: true }] }),
    "thinking",
  );
  assert.equal(
    voiceAuraPhase({ ...live, tasks: [{ state: "claimed" }] }),
    "thinking",
  );
  assert.equal(
    voiceAuraPhase({ ...live, tasks: [{ state: "completed" }] }),
    "listening",
  );
  // Talking again over queued work is listening, not thinking.
  assert.equal(
    voiceAuraPhase({
      ...live,
      tasks: [{ state: "queued" }],
      captions: [{ speaker: "user", complete: false }],
    }),
    "listening",
  );
  assert.equal(
    voiceAuraPhase({
      ...live,
      captions: [
        { speaker: "user", complete: true },
        { speaker: "assistant", complete: true },
      ],
    }),
    "listening",
  );
});

test("hold to talk listens only while held, and mute dims the aura", () => {
  const held = { ...live, inputMode: "push_to_talk" as const };
  assert.equal(voiceAuraPhase(held), "resting");
  assert.equal(voiceAuraPhase({ ...held, holding: true }), "listening");
  assert.equal(
    voiceAuraPhase({ ...held, holding: true, microphoneBlocked: true }),
    "resting",
  );
  assert.equal(
    voiceAuraPhase({ ...live, muted: true, microphoneBlocked: true }),
    "muted",
  );
  // The assistant speaking wins over a held button release race.
  assert.equal(
    voiceAuraPhase({ ...held, holding: true, outputActive: true }),
    "speaking",
  );
});

test("status text names every phase for VoiceOver", () => {
  assert.equal(voiceAuraStatus("connecting", "automatic"), "Connecting…");
  assert.equal(voiceAuraStatus("listening", "automatic"), "Listening");
  assert.equal(voiceAuraStatus("thinking", "automatic"), "Thinking");
  assert.equal(voiceAuraStatus("speaking", "automatic"), "Speaking");
  assert.equal(voiceAuraStatus("muted", "automatic"), "Muted");
  assert.equal(voiceAuraStatus("resting", "push_to_talk"), "Ready");
  assert.equal(voiceAuraStatus("resting", "automatic"), "Connected");
});

test("levels drive listening and speaking; Reduce Motion keeps only opacity", () => {
  assert.equal(voiceAuraTargets("listening", false).source, "input");
  assert.equal(voiceAuraTargets("speaking", false).source, "output");
  assert.ok(voiceAuraTargets("listening", false).gain > 0);
  assert.ok(voiceAuraTargets("speaking", false).gain > 0);
  assert.equal(voiceAuraTargets("thinking", false).gain, 0);
  assert.ok(
    voiceAuraTargets("thinking", false).orbit >
      voiceAuraTargets("listening", false).orbit,
  );
  assert.ok(
    voiceAuraTargets("muted", false).opacity <
      voiceAuraTargets("resting", false).opacity,
  );
  for (const phase of [
    "resting",
    "connecting",
    "listening",
    "thinking",
    "speaking",
    "muted",
  ] as const) {
    const reduced = voiceAuraTargets(phase, true);
    assert.deepEqual(
      { scale: reduced.scale, gain: reduced.gain, orbit: reduced.orbit, breathe: reduced.breathe },
      { scale: 1, gain: 0, orbit: 0, breathe: false },
    );
    assert.equal(reduced.opacity, voiceAuraTargets(phase, false).opacity);
  }
});

test("levels rise quickly and fall slowly", () => {
  assert.ok(levelTransitionMs(0.1, 0.8) < levelTransitionMs(0.8, 0.1));
});

test("playback activity has hysteresis so pauses between words keep speaking", () => {
  let state = nextOutputActivity({ active: false }, 0.08, 0);
  assert.equal(state.active, false);
  state = nextOutputActivity(state, 0.4, 0);
  assert.equal(state.active, true);
  state = nextOutputActivity(state, 0.08, 50);
  assert.deepEqual(state, { active: true });
  state = nextOutputActivity(state, 0.01, 100);
  assert.deepEqual(state, { active: true, quietSince: 100 });
  state = nextOutputActivity(state, 0, 100 + OUTPUT_SPEAKING_RELEASE_MS - 1);
  assert.equal(state.active, true);
  state = nextOutputActivity(state, 0, 100 + OUTPUT_SPEAKING_RELEASE_MS);
  assert.deepEqual(state, { active: false });
  // A word during the quiet window resets the release.
  state = nextOutputActivity({ active: true, quietSince: 0 }, 0.3, 300);
  assert.deepEqual(state, { active: true });
});
