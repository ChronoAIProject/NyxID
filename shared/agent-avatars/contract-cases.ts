import validationFixture from "./validation-fixture.json";
/** Executed by both web and native test runners. Assertions target product contracts. */
import {
  AVATAR_CATALOGUE,
  isValidAvatarForKind,
  chooseAvatar,
  defaultAvatar,
  isValidAvatar,
  randomAvatar,
  resolveAvatar,
  sameStoredAvatar,
  type AvatarConfig,
} from "./model";
import {
  drawAvatar,
  faceMask,
  FACE_GEOMETRY,
  maskCacheInfo,
  coarseEyeHoles,
  resolveFaceSurface,
  textureGrid,
  contrastRatio,
  stillCacheInfo,
} from "./draw";
import { resolveProcess, reactionFor } from "./process";
export interface Check {
  (condition: unknown, message: string): void;
}
function holes(sample: (u: number, v: number) => boolean): number {
  const N = 192,
    seen = new Uint8Array(N * N);
  let count = 0;
  for (let y = 0; y < N; y++)
    for (let x = 0; x < N; x++) {
      const start = y * N + x;
      if (seen[start] || sample((x + 0.5) / N, (y + 0.5) / N)) continue;
      const todo = [start];
      seen[start] = 1;
      let outer = false;
      while (todo.length) {
        const at = todo.pop()!,
          px = at % N,
          py = Math.floor(at / N);
        if (!px || !py || px === N - 1 || py === N - 1) outer = true;
        for (const [dx, dy] of [
          [-1, 0],
          [1, 0],
          [0, -1],
          [0, 1],
        ]) {
          const nx = px + dx!,
            ny = py + dy!;
          if (nx < 0 || ny < 0 || nx >= N || ny >= N) continue;
          const key = ny * N + nx;
          if (!seen[key] && !sample((nx + 0.5) / N, (ny + 0.5) / N)) {
            seen[key] = 1;
            todo.push(key);
          }
        }
      }
      if (!outer) count++;
    }
  return count;
}
export function runFaceContracts(check: Check) {
  for (const row of validationFixture)
    check(isValidAvatarForKind(row.avatar, row.kind) === row.valid, row.name);
  const nyx = defaultAvatar("nyx", "nyxbot");
  check(
    nyx.choices.head === "round" &&
      nyx.choices.eyes === "tall" &&
      nyx.choices.palette === "core-nyx" &&
      nyx.choices.treatment === "gradient-face" &&
      nyx.choices.background === "treatment" &&
      nyx.choices.frame === "tile",
    "latest purple default",
  );
  check(Object.keys(AVATAR_CATALOGUE.fields.palette).length === 35, "35 pairs");
  check(
    Object.keys(AVATAR_CATALOGUE.fields.ripple_palette).length === 12,
    "12 ripple palettes",
  );
  check(
    Object.keys(AVATAR_CATALOGUE.fields.surface).length === 19,
    "all textured variants",
  );
  for (const value of [0, 0.1, 0.5, 0.99, 1])
    for (const kind of ["nyxbot", "specialist"])
      check(
        isValidAvatar(randomAvatar(() => value, kind)),
        `constant random ${value} ${kind}`,
      );
  for (let i = 0; i < 1000; i++)
    check(isValidAvatar(randomAvatar()), "random is savable");
  const ascii: AvatarConfig = {
    template: "face-v1",
    choices: {
      ...nyx.choices,
      surface: "ascii-braille",
      background: "mesh",
      react_thinking: "character-churn",
    },
  };
  const changed = chooseAvatar(ascii, "treatment", "three-stop");
  check(
    changed.choices.surface === "ascii-braille" &&
      changed.choices.background === "mesh" &&
      changed.choices.react_thinking === "character-churn",
    "independent choices survive treatment",
  );
  const future = {
    template: "face-v9",
    choices: { new_field: "new-value", head: "round" },
  };
  check(
    resolveAvatar({ id: "n", kind: "nyxbot", avatar: future }).choices.head ===
      "round",
    "future fallback",
  );
  check(
    future.choices.new_field === "new-value" && future.template === "face-v9",
    "no write on fallback",
  );
  check(
    sameStoredAvatar(
      { template: "face-v9", choices: { b: "x", a: "y" } },
      { template: "face-v9", choices: { a: "y", b: "x" } },
    ),
    "semantic future envelope equality",
  );
  check(
    resolveAvatar({
      id: "n",
      kind: "nyxbot",
      avatar: { template: "face-v1", choices: { head: "round", eyes: "wink" } },
    }).choices.head === "round",
    "saved Nyx head honored",
  );
  const appearances = new Set<string>();
  for (const treatment of Object.keys(AVATAR_CATALOGUE.fields.treatment))
    appearances.add(
      JSON.stringify(
        drawAvatar(
          {
            template: "face-v1",
            choices: { ...nyx.choices, treatment },
          },
          { resting: true, size: 112 },
        ),
      ),
    );
  check(appearances.size === 11, "all 11 treatments produce distinct output");
  for (const head of ["block", "tall", "wide", "round", "nyxbot"])
    for (const eyes of ["dots", "tall", "calm", "happy", "wink", "visor"]) {
      const mask = faceMask(head, eyes);
      check(
        holes(mask.sample) === (eyes === "visor" ? 1 : 2),
        `${head}/${eyes} aperture topology`,
      );
    }
  for (const head of ["block", "tall", "wide", "round", "nyxbot"])
    for (const eyes of ["dots", "tall", "calm", "happy", "wink", "visor"])
      for (const surface of Object.keys(AVATAR_CATALOGUE.fields.surface).filter(
        (s) => s !== "flat",
      )) {
        const resolved = resolveFaceSurface(surface, head, eyes, 112),
          grid = textureGrid(surface)!,
          pad = surface.startsWith("ascii-")
            ? 0.02
            : head === "nyxbot"
              ? 0.14
              : 0.12;
        const count = coarseEyeHoles(head, eyes, grid.columns, grid.rows, pad),
          expected = eyes === "visor" ? 1 : 2;
        check(
          count === expected
            ? resolved.surface === surface
            : resolved.surface === "flat" && resolved.fallback === "eye-grid",
          `${head}/${eyes}/${surface} safe coarse analogue`,
        );
      }
  for (const [treatment, background, theme] of [
    ["solid", "dark", "dark"],
    ["pastel", "solid", "light"],
    ["solid", "transparent", "dark"],
    ["solid", "transparent", "light"],
  ] as const) {
    const config = {
        template: "face-v1",
        choices: { ...nyx.choices, treatment, background },
      },
      scene = drawAvatar(config, { size: 112, resting: true, theme });
    const color = scene.paths.find((p) => p.clip === "face")?.fill;
    const backdrop =
      background === "dark"
        ? "#0c0c0e"
        : background === "solid"
          ? "#b48cff"
          : theme === "light"
            ? "#ffffff"
            : "#141416";
    check(
      !!color && contrastRatio(color, backdrop) >= 2.5,
      `${treatment}/${background}/${theme} readable foreground`,
    );
    check(
      config.choices.treatment === treatment &&
        config.choices.background === background,
      "contrast does not mutate preferences",
    );
  }
  check(
    !Object.keys(AVATAR_CATALOGUE.fields.palette).some((id) =>
      /(?:^|-)red(?:-|$)/.test(id),
    ),
    "no destructive red identity palette",
  );
  // Check actual RGB endpoints; names alone cannot prevent a destructive-red ink.
  for (const [id, palette] of Object.entries(FACE_GEOMETRY.palettes)) {
    for (const color of palette.colors) {
      const [r, g, b] = [1, 3, 5].map(
        (at) => parseInt(color.slice(at, at + 2), 16) / 255,
      ) as [number, number, number];
      const high = Math.max(r, g, b),
        low = Math.min(r, g, b),
        chroma = high - low;
      const hue =
        chroma === 0
          ? 0
          : ((high === r
              ? (g - b) / chroma
              : high === g
                ? (b - r) / chroma + 2
                : (r - g) / chroma + 4) *
              60 +
              360) %
            360;
      check(
        !(
          chroma >= 0.15 &&
          chroma / Math.max(high, 0.001) >= 0.45 &&
          (hue <= 15 || hue >= 350)
        ),
        `${id}/${color}: no destructive-red RGB endpoint`,
      );
    }
  }
  for (let i = 0; i < 6; i++) {
    const palette = [
      "core-violet",
      "core-ocean",
      "core-mint",
      "core-ember",
      "core-indigo",
      "core-graphite",
    ][i]!;
    const config = {
      template: "face-v1",
      choices: {
        head: "tall",
        eyes: "dots",
        palette,
        treatment: "tint",
        background: "treatment",
      },
    };
    const dark = drawAvatar(config, { kind: "specialist", theme: "dark" }),
      light = drawAvatar(config, { kind: "specialist", theme: "light" });
    check(
      [...dark.backgroundPaths, ...dark.paths].some(
        (p) => p.clip === "face" && p.fill === FACE_GEOMETRY.tints[i]!.fg,
      ),
      "original dark tint",
    );
    check(
      [...light.backgroundPaths, ...light.paths].some(
        (p) => !p.clip && p.fill === FACE_GEOMETRY.tints[i]!.solid,
      ) &&
        [...light.backgroundPaths, ...light.paths].some(
          (p) => p.clip === "face" && p.fill === "#ffffff",
        ),
      "original light solid",
    );
  }
  check(
    resolveProcess({ surface: "turn", turn: { pendingOwnerAction: true } })
      .state === "approval",
    "approval without turn",
  );
  check(
    resolveProcess({
      surface: "turn",
      turn: {
        pendingOwnerAction: true,
        activeTurn: { stopRequested: true, activityStatuses: ["running"] },
      },
    }).state === "stopping",
    "stop is not settled",
  );
  check(
    resolveProcess({
      surface: "voice",
      voice: { inputAccepting: false, outputPlaying: false },
      turn: { activeTurn: { activityStatuses: ["running"] } },
      connection: { status: "reconnecting" },
    }).state === "idle",
    "voice does not infer task/mic",
  );
  check(
    resolveProcess({
      surface: "voice",
      voice: { inputAccepting: true, outputPlaying: false },
      connection: { status: "reconnecting" },
    }).connection === "reconnecting",
    "transport remains independent",
  );
  check(
    resolveProcess({
      surface: "turn",
      turn: { latestReply: "stopped", celebrating: true },
    }).state === "idle",
    "cancellation is not failure/success",
  );
  const saved = {
    template: "face-v1",
    choices: { ...nyx.choices, react_thinking: "character-churn" },
  };
  check(
    reactionFor(saved, "thinking", 112).effective === "thinking-pulse",
    "vector analogue",
  );
  check(
    reactionFor(
      { ...saved, choices: { ...saved.choices, surface: "ascii-shaded" } },
      "thinking",
      112,
    ).effective === "character-churn",
    "character reaction",
  );
  check(
    reactionFor(saved, "thinking", 44).effective === "thinking-pulse",
    "quiet analogue",
  );
  check(
    reactionFor(saved, "connecting", 112, false, true).effective === "still",
    "reduced scan shows complete frame",
  );
  for (const surface of Object.keys(AVATAR_CATALOGUE.fields.surface)) {
    const config = {
      template: "face-v1",
      choices: { ...nyx.choices, surface },
    };
    const still = drawAvatar(config, { size: 112, native: true });
    check(
      (still.paths.length > 0 || !!still.foregroundPlane) &&
        still.paths.every((p) => p.d && !p.d.includes("NaN")),
      "bounded valid paths",
    );
    for (const size of [16, 20, 24, 32, 44, 80]) {
      const compact = drawAvatar(config, { size, native: true });
      check(
        compact.surface === still.surface,
        "saved texture survives at app sizes",
      );
      check(
        compact.mask === still.mask && compact.transform === still.transform,
        "compact appearance preserves the authored face framing and eyes",
      );
    }
    check(
      drawAvatar(config, { size: 112, native: true, eyeFrame: "closed" })
        .mask === still.mask || surface === "flat",
      "rich eyes stay at exact sampled frame",
    );
  }
  check(
    maskCacheInfo().entries <= maskCacheInfo().maxEntries &&
      stillCacheInfo().entries <= stillCacheInfo().maxEntries,
    "bounded caches",
  );
}
