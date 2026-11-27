import fixture from "./activity-fixture.json";
import {
  agentRosterMood,
  resolveTurnActivity,
  ROSTER_ACTIVITY_STATES,
  type AgentRosterMoodInput,
} from "./activity";
import { drawAvatar } from "./draw";
import { AVATAR_MOODS, defaultAvatar } from "./model";
import { avatarPresentation } from "./presentation";
import { quietActivityMotion, sampleActivityMotion } from "./quiet-motion";
import { contours } from "./svg-path.js";

type Check = (ok: unknown, message: string) => void;

export function runTurnActivityContracts(check: Check) {
  for (const row of fixture.turns) {
    const actual = resolveTurnActivity(row.turn, {
      pendingOwnerAction: row.pending_owner_action,
      streamedTextPresent: row.streamed_text_present,
    });
    check(
      actual === row.expected,
      `${row.name}: expected ${row.expected}, got ${actual}`,
    );
  }
}

export function runRosterActivityContracts(check: Check) {
  for (const row of fixture.aggregation) {
    const actual = agentRosterMood(row.input as AgentRosterMoodInput);
    check(
      actual === row.expected,
      `${row.name}: expected ${row.expected}, got ${actual}`,
    );
  }
  const expectedRank = [
    "approval",
    "working",
    "drafting",
    "retry",
    "thinking",
    "waiting",
    "stopping",
  ] as const;
  for (const higher of expectedRank) {
    for (const lower of expectedRank.slice(expectedRank.indexOf(higher) + 1)) {
      const a = { conversation_id: "a", state: higher };
      const b = { conversation_id: "b", state: lower };
      check(
        agentRosterMood({ agentId: "agent", activity: [a, b] }) === higher,
        `${higher} outranks ${lower}`,
      );
      check(
        agentRosterMood({ agentId: "agent", activity: [b, a] }) === higher,
        "rank is independent of snapshot order",
      );
    }
  }
}

export function runBadgeContracts(check: Check) {
  const config = defaultAvatar("test", "specialist");
  const expected = {
    thinking: "primary",
    working: "primary",
    drafting: "primary",
    waiting: "neutral",
    approval: "warning",
    retry: "neutral",
    stopping: "neutral",
    success: "success",
    failure: "neutral",
  } as const;
  const paths = new Set<string>();
  for (const state of AVATAR_MOODS) {
    const compact = avatarPresentation(state, { config, size: 20 });
    const rail = avatarPresentation(state, { config, size: 24 });
    const reduced = avatarPresentation(state, {
      config,
      size: 56,
      reduced: true,
      native: true,
    });
    check(
      compact.state === state && compact.motion.mode === "static",
      `${state} compact state and motion`,
    );
    check(reduced.motion.mode === "static", `${state} reduced motion`);
    check(
      compact.badge === rail.badge && rail.badge === reduced.badge,
      `${state} independent badge geometry`,
    );
    if (!(state in expected)) {
      check(compact.badge === null, `${state} has no badge`);
      continue;
    }
    const badge = compact.badge;
    check(
      badge?.tone === expected[state as keyof typeof expected],
      `${state} uses its semantic tone`,
    );
    if (!badge) continue;
    check(
      !paths.has(badge.glyph.d) && badge.glyph.d.length > 0,
      `${state} has its own glyph`,
    );
    check(
      badge.glyph.strokeWidth === undefined || badge.glyph.strokeWidth >= 2,
      `${state} uses legible strokes`,
    );
    const points = contours(badge.glyph.d).flat();
    const radius = (badge.glyph.strokeWidth ?? 0) / 2;
    const bounds = [0, 1].map((axis) => {
      const values = points.map((point) => point[axis]!);
      return [
        Math.min(...values) - radius,
        Math.max(...values) + radius,
      ] as const;
    });
    check(
      bounds.every(([min, max]) => min >= 3 && max <= 21),
      `${state} glyph stays inside the disc margin`,
    );
    check(
      bounds.some(([min, max]) => max - min >= 12),
      `${state} glyph spans at least half the disc`,
    );
    paths.add(badge.glyph.d);
  }
  check(paths.size === 9, "all nine drawn badges use distinct glyphs");
  const success = avatarPresentation("success", { config, size: 56 });
  const failure = avatarPresentation("failure", { config, size: 56 });
  check(
    success.oneShot && success.staticFrame === "happy",
    "success has one happy celebration",
  );
  check(
    !failure.oneShot && failure.staticFrame === "calm",
    "failure settles to a calm frame",
  );
  check(
    avatarPresentation("approval", { config, size: 20 }).staticFrame === "tall",
    "approval stays attentive",
  );
  check(
    avatarPresentation("stopping", { config, size: 20 }).staticFrame === "calm",
    "stopping stays calm",
  );
}

export function runQuietMotionContracts(check: Check) {
  const config = defaultAvatar("test", "specialist");
  const size = 56;
  const radius = size / 2;
  const channels = ["sway", "lift", "tilt", "scale"] as const;
  function pixelMovement({
    x,
    y,
    rotate,
    scale,
  }: ReturnType<typeof drawAvatar>["motion"]) {
    return {
      sway: (Math.abs(x) * size) / 512,
      lift: (Math.abs(y) * size) / 512,
      tilt: 2 * radius * Math.sin((Math.abs(rotate) * Math.PI) / 360),
      scale: Math.abs(scale - 1) * radius,
    };
  }
  const profiles = [];
  const idlePeak = pixelMovement(
    drawAvatar(config, { size, seconds: Math.PI / 4.8 }).motion,
  ).scale;
  for (const state of ROSTER_ACTIVITY_STATES) {
    const presentation = avatarPresentation(state, { config, size });
    const signature = quietActivityMotion(state);
    check(
      presentation.motion.mode ===
        (state === "stopping" ? "static" : "quiet") &&
        presentation.motion.signature === signature &&
        signature,
      `${state} declares its quiet profile`,
    );
    if (!signature) continue;
    const period = signature.period;
    const samples = [0, 0.25, 0.5, 0.75, 1].map(
      (phase) => phase * (period ?? 6),
    );
    const peaks = { sway: 0, lift: 0, tilt: 0, scale: 0 };
    for (const surface of ["flat", "dither-chunky"]) {
      const appearance = { ...config, choices: { ...config.choices, surface } };
      for (const seconds of samples) {
        const drawn = drawAvatar(appearance, {
          size,
          mood: state,
          seconds,
        });
        const { transform, ringOpacity } = sampleActivityMotion(
          signature,
          seconds,
        );
        check(
          drawn.surface === surface,
          `${state} keeps ${surface} during quiet motion`,
        );
        check(
          drawn.motion.x === transform.x &&
            drawn.motion.y === transform.y &&
            drawn.motion.rotate === transform.rotate &&
            drawn.motion.scale === transform.scale,
          `${state} uses its quiet transform`,
        );
        check(
          drawn.paths.filter((path) => path.role === "activity-ring").length ===
            1 &&
            drawn.paths.find((path) => path.role === "activity-ring")
              ?.opacity === ringOpacity,
          `${state} uses its quiet ring opacity`,
        );
        const displacement = pixelMovement(drawn.motion);
        for (const channel of channels) {
          peaks[channel] = Math.max(peaks[channel], displacement[channel]);
        }
      }
    }
    const ringMoves = signature.ringRange[0] !== signature.ringRange[1];
    for (const peak of Object.values(peaks)) {
      check(peak === 0 || peak >= 1, `${state} uses motion visible at 56 px`);
    }
    if (state === "waiting" || state === "stopping") {
      check(
        Object.values(peaks).every((peak) => peak === 0),
        `${state} keeps the face still`,
      );
      check(
        ringMoves === (state === "waiting"),
        `${state} has the expected halo behavior`,
      );
    } else {
      check(
        Math.max(...Object.values(peaks)) > idlePeak,
        `${state} peaks above idle breathing`,
      );
    }
    profiles.push({ state, period, peaks, ringMoves });
    const rich = avatarPresentation(state, { config, size: 96 });
    check(
      rich.motion.mode === "rich" && rich.motion.signature === null,
      `${state} retains its rich reaction`,
    );
    const quiet = {
      ...config,
      choices: { ...config.choices, motion: "quiet" },
    };
    check(
      avatarPresentation(state, { config: quiet, size: 112 }).motion
        .signature === signature,
      `${state} follows saved Quiet at large sizes`,
    );
  }
  for (const [index, first] of profiles.entries()) {
    for (const second of profiles.slice(index + 1)) {
      const periodDistinct =
        first.period === null || second.period === null
          ? first.ringMoves !== second.ringMoves
          : Math.max(first.period, second.period) /
              Math.min(first.period, second.period) >=
            1.5;
      const movementDistinct = channels.some(
        (channel) =>
          Math.abs(first.peaks[channel] - second.peaks[channel]) >= 1,
      );
      check(
        periodDistinct || movementDistinct,
        `${first.state} and ${second.state} differ by visible motion or cadence`,
      );
    }
  }

  const still = { ...config, choices: { ...config.choices, motion: "still" } };
  for (const options of [
    { size: 20 },
    { size: 56, reduced: true },
    { size: 512, resting: true },
  ]) {
    const frame = drawAvatar(config, { ...options, mood: "working" });
    check(
      !frame.paths.some((path) => path.role === "activity-ring"),
      "static and export/resting frames omit the live ring role",
    );
    check(
      frame === drawAvatar(config, { ...options, mood: "working", seconds: 1 }),
      "static draws reuse the same resting frame",
    );
  }
  check(
    avatarPresentation("working", { config: still, size: 56 }).motion.mode ===
      "static",
    "saved Still disables quiet motion",
  );
  check(
    drawAvatar(still, { size: 56, mood: "working", seconds: 1 }).motion
      .scale === 1,
    "saved Still disables the sampled transform",
  );

  const reactionStill = {
    ...config,
    choices: { ...config.choices, react_thinking: "still" },
  };
  const stillFrame = drawAvatar(reactionStill, { size, mood: "thinking" });
  check(
    avatarPresentation("thinking", { config: reactionStill, size }).motion
      .signature === null &&
      !stillFrame.paths.some((path) => path.role === "activity-ring") &&
      drawAvatar(reactionStill, { size, mood: "thinking", seconds: 1.5 }) ===
        stillFrame,
    "saved per-state Still disables activity motion",
  );
  for (const [mood, size] of [
    ["working", 112],
    ["idle", 56],
    ["failure", 56],
  ] as const) {
    check(
      !drawAvatar(config, { size, mood }).paths.some(
        (path) => path.role === "activity-ring",
      ),
      `${mood} without a quiet signature has no activity ring role`,
    );
  }
  const movingLight = {
    ...config,
    choices: {
      ...config.choices,
      surface: "dither-chunky",
      react_thinking: "moving-light",
    },
  };
  const quiet = drawAvatar(movingLight, {
    size,
    mood: "thinking",
    seconds: 1.5,
  });
  check(
    quiet.motion.scale >= 1.04,
    "saved Moving light uses the thinking profile at quiet sizes",
  );
  check(
    drawAvatar(movingLight, { size: 112, mood: "thinking", seconds: 1.5 })
      .reaction === "moving-light",
    "saved Moving light keeps its rich reaction",
  );
}
