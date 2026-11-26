/** Extract only the reference's genuine source declarations. Never execute its DOM wrapper. */
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import vm from "node:vm";
import { contours } from "./svg-path.js";
const root = new URL("../../", import.meta.url);
const source = readFileSync(
  new URL("mockups/nyxid-bot-faces-reference.html", root),
  "utf8",
);
const sha = createHash("sha256").update(source).digest("hex");
if (sha !== "9d91953d4e8a97a190ae75ce32f3b9acec1cb94e4024914b8d618a95ef30ea2e")
  throw Error("Reference changed; review source before regenerating");
const slice = (a, b) =>
  source.slice(source.indexOf(a), source.indexOf(b, source.indexOf(a)));
const kit = vm.runInNewContext(
  slice("const TINTS =", "/* ---------------- rendering helpers") +
    ";({TINTS,HEADS,EYES,LOGO,NYXBOT_EYES,head,compose,circle,rrect})",
  {},
  { timeout: 1000 },
);
const sets = vm.runInNewContext(
  slice("const SETS =", "const PAIRS =") + ";SETS",
  {},
  { timeout: 1000 },
);
const ripples = vm.runInNewContext(
  slice("const RIPPLE_PALS =", "let bannerPal") + ";RIPPLE_PALS",
  {},
  { timeout: 1000 },
);
const label = (id) =>
  id
    .split("-")
    .map((s) => s[0].toUpperCase() + s.slice(1))
    .join(" ");
const options = (ids) =>
  Object.fromEntries(ids.map((id) => [id, { label: label(id) }]));
const eyes = kit.EYES.map((e) => e.name),
  heads = kit.HEADS.map((h) => h.name);
const treatments = [
  "tint",
  "solid",
  "gradient-tile",
  "three-stop",
  "mesh",
  "conic-sheen",
  "glass",
  "gradient-face",
  "night-glow",
  "pastel",
  "neon-line",
];
const surfaces = [
  "flat",
  "dither-ordered",
  "dither-chunky",
  "dither-fine",
  "dither-diffusion",
  "dither-halftone",
  "dither-phosphor",
  "dither-handheld",
  "dither-grain",
  "dither-duotone",
  "ascii-shaded",
  "ascii-blocks",
  "ascii-braille",
  "ascii-binary",
  "ascii-name",
  "ascii-outline",
  "ascii-gradient",
  "ascii-hatching",
  "ascii-inverted",
];
const backgrounds = [
  "treatment",
  "tint",
  "transparent",
  "solid",
  "gradient",
  "three-stop",
  "mesh",
  "conic",
  "glass",
  "dark",
  "glow",
  "pastel",
  "ripple",
];
const states = [
  "idle",
  "thinking",
  "working",
  "drafting",
  "waiting",
  "approval",
  "listening",
  "speaking",
  "connecting",
  "reconnecting",
  "connected",
  "stopping",
  "success",
  "failure",
  "created",
  "choosing",
  "retry",
];
const defaults = {
  idle: "blink",
  thinking: "character-churn",
  working: "rain",
  drafting: "typing",
  waiting: "slow-pulse",
  approval: "attentive",
  listening: "halftone-ripple",
  speaking: "output-pulse",
  connecting: "scan",
  reconnecting: "channel-offset",
  connected: "connection-ring",
  stopping: "settling",
  success: "hop",
  failure: "concerned",
  created: "pixel-assembly",
  choosing: "face-shuffle",
  retry: "slow-pulse",
};
const reactions = {};
for (const state of states) {
  for (const id of [defaults[state], "still"]) {
    const rich = [
      "character-churn",
      "rain",
      "typing",
      "scan",
      "face-shuffle",
    ].includes(id)
      ? "character"
      : ["moving-light", "pixel-assembly"].includes(id)
        ? "dither"
        : "vector";
    reactions[id] ??= {
      label: label(id),
      states: [],
      family: rich,
      vector:
        id === "still"
          ? "still"
          : state === "failure"
            ? "concerned"
            : state === "success"
              ? "happy"
              : "pulse",
      quiet:
        id === "still"
          ? "still"
          : state === "failure"
            ? "concerned"
            : state === "success"
              ? "happy"
              : state === "approval"
                ? "attentive"
                : "pulse",
      static:
        state === "failure"
          ? "calm"
          : state === "success"
            ? "happy"
            : state === "stopping"
              ? "calm"
              : state === "approval"
                ? "tall"
                : "rest",
      oneShot: ["connected", "success", "created", "choosing"].includes(state),
    };
    reactions[id].states.push(state);
  }
}
reactions["moving-light"] = {
  label: "Moving light",
  states: ["idle", "thinking", "working"],
  family: "dither",
  vector: "pulse",
  quiet: "pulse",
  static: "rest",
  oneShot: false,
};
reactions["blink"].states.push("waiting");
const analogues = {
  blink: ["vector", "blink", "blink"],
  "moving-light": ["dither", "pulse", "blink"],
  "character-churn": ["character", "thinking-pulse", "thinking-pulse"],
  rain: ["character", "working-accent", "working-accent"],
  typing: ["character", "drafting-pulse", "drafting-pulse"],
  "slow-pulse": ["vector", "slow-pulse", "slow-pulse"],
  attentive: ["vector", "attentive", "attentive"],
  "halftone-ripple": ["vector", "halftone-ripple", "listening-ring"],
  "output-pulse": ["vector", "output-pulse", "output-pulse"],
  scan: ["character", "connecting-pose", "connecting-pose"],
  "channel-offset": ["vector", "channel-offset", "reconnecting-pose"],
  "connection-ring": ["vector", "connection-ring", "connected-accent"],
  settling: ["vector", "settling", "stopping-pose"],
  hop: ["vector", "hop", "happy"],
  concerned: ["vector", "concerned", "concerned"],
  "pixel-assembly": ["dither", "assembly-pulse", "assembly-pulse"],
  "face-shuffle": ["dither", "face-shuffle", "assembly-pulse"],
};
for (const [id, [family, vector, quiet]] of Object.entries(analogues))
  Object.assign(reactions[id], { family, vector, quiet });
reactions.scan.oneShot = true;
reactions["channel-offset"].oneShot = true;
for (const [id, reaction] of Object.entries(reactions))
  reaction.duration =
    {
      scan: 2.2,
      "pixel-assembly": 1.8,
      "face-shuffle": 1.6,
      "channel-offset": 0.8,
      "connection-ring": 1.2,
    }[id] ?? 1.6;
const paletteChoices = {},
  palettes = {};
for (const set of sets)
  for (const p of set.colours) {
    const id = `${set.name}-${p.name}`.toLowerCase();
    paletteChoices[id] = {
      label: p.name,
      group: set.name,
      ...(id === "core-nyx" ? { kinds: ["nyxbot"] } : {}),
    };
    palettes[id] = { ...paletteChoices[id], colors: p.c };
  }
const fields = {
  eyes: options(eyes),
  head: { ...options(heads), logo: { label: "Nyx", kinds: ["nyxbot"] } },
  palette: paletteChoices,
  treatment: options(treatments),
  surface: options(surfaces),
  background: options(backgrounds),
  frame: options(["circle", "tile", "none"]),
  motion: options(["automatic", "quiet", "still"]),
  ripple_palette: options(ripples.map((p) => p.name.toLowerCase())),
};
const surfaceLabels = [
  "Flat",
  "Ordered, 1-bit",
  "Chunky",
  "Fine",
  "Error diffusion",
  "Halftone",
  "Phosphor screen",
  "Four-tone handheld",
  "Film grain",
  "Duotone field",
  "Shaded",
  "Blocks",
  "Braille",
  "Binary",
  "Name fill",
  "Outline",
  "Gradient",
  "Hatching",
  "Inverted",
];
surfaces.forEach((id, i) => {
  fields.surface[id].label = surfaceLabels[i];
  fields.surface[id].group =
    i === 0 ? "Vector" : i <= 9 ? "Dither" : "Character";
});
fields.background.treatment.label = "Treatment background";
fields.treatment.tint.label = "Tint on dark";
fields.treatment["three-stop"].label = "Three-stop";
fields.treatment["gradient-tile"].label = "Gradient tile";
fields.treatment["conic-sheen"].label = "Conic sheen";
fields.treatment["gradient-face"].label = "Gradient face";
fields.treatment["night-glow"].label = "Night glow";
fields.treatment["neon-line"].label = "Neon line";
for (const state of states)
  fields[`react_${state}`] = options(
    Object.keys(reactions).filter((id) => reactions[id].states.includes(state)),
  );
const catalog = {
  template: "face-v1",
  version: 1,
  reference_sha256: sha,
  max_choices: 32,
  max_identifier: 64,
  fields,
  reactions,
  defaults: {
    head: "round",
    eyes: "tall",
    palette: "core-nyx",
    treatment: "gradient-face",
    surface: "flat",
    background: "treatment",
    frame: "tile",
    motion: "automatic",
    ripple_palette: "nyx",
  },
  default_reactions: defaults,
  compatibility: {
    stroke_treatment: "neon-line",
    stroke_surface: "flat",
    logo_kinds: ["nyxbot"],
    nyx_palette_kinds: ["nyxbot"],
  },
};
const geometry = {
  version: 1,
  reference_sha256: sha,
  heads: {},
  tints: kit.TINTS,
  palettes,
  ripples: Object.fromEntries(ripples.map((p) => [p.name.toLowerCase(), p])),
  bayer: vm.runInNewContext(slice("const B8 =", "const bay =") + ";B8"),
  effects: {
    dither_grids: [48, 32, 64, 72, 26, 48, 40, 96, 56],
    ascii_columns: [34, 26, 36, 30, 30, 30, 32, 30, 30],
    ascii_ramp: " .:-=+*#%@",
    churn_chars: "%#*+=-:.01",
    ascii_pad: 0.02,
    dither_pad: 0.12,
    nyx_dither_pad: 0.14,
  },
};
for (const h of ["nyxbot", ...heads]) {
  const index = heads.indexOf(h),
    spec = kit.HEADS[index],
    head =
      h === "nyxbot"
        ? kit.LOGO
        : kit.head(
            (24 - spec.w) / 2,
            (24 - spec.h) / 2 - 0.4,
            spec.w,
            spec.h,
            spec.round,
          );
  const viewBox = h === "nyxbot" ? [-36, -36, 330] : [0, 0, 24],
    eyePaths = {};
  for (let i = 0; i < eyes.length; i++) {
    let d;
    if (h === "nyxbot") {
      if (i === 0) d = kit.NYXBOT_EYES;
      else {
        // Apply the specialist eye design to the original NyxBot anchors/radius.
        const scale = ["happy", "wink"].includes(eyes[i])
            ? 42 / 4.8
            : eyes[i] === "calm"
              ? 42 / 4.6
              : 21 / 1.9,
          raw = kit.EYES[i].draw(0, 54 / scale, 0);
        d = contours(raw)
          .map(
            (p) =>
              p
                .map(
                  ([x, y], j) =>
                    `${j ? "L" : "M"}${+(x * scale + 166).toFixed(5)} ${+(y * scale + 86).toFixed(5)}`,
                )
                .join("") + "Z",
          )
          .join("");
      }
    } else {
      const x = (24 - spec.w) / 2,
        y = (24 - spec.h) / 2 - 0.4;
      d = kit.EYES[i].draw(
        x + spec.w * 0.3,
        x + spec.w * 0.7,
        y + spec.h * 0.56,
      );
    }
    eyePaths[eyes[i]] = { d, contours: contours(d) };
  }
  geometry.heads[h] = {
    d: head,
    viewBox,
    contours: contours(head),
    eyes: eyePaths,
  };
}
// Five decimals: <0.00022 export pixels even in the 24-unit specialist space.
for (const h of Object.values(geometry.heads)) {
  h.contours = h.contours.map((p) =>
    p.map((pt) => pt.map((n) => +n.toFixed(5))),
  );
  for (const e of Object.values(h.eyes))
    e.contours = e.contours.map((p) =>
      p.map((pt) => pt.map((n) => +n.toFixed(5))),
    );
}
const outputs = { "catalog.json": catalog, "geometry.json": geometry };
let different = false;
for (const [file, value] of Object.entries(outputs)) {
  const text =
      JSON.stringify(value, null, file === "catalog.json" ? 2 : 0) + "\n",
    url = new URL(file, import.meta.url);
  if (process.argv.includes("--check")) {
    if (readFileSync(url, "utf8") !== text) {
      console.error(`${file} is stale`);
      different = true;
    }
  } else writeFileSync(url, text);
}
if (different) process.exit(1);
console.log(
  `face-v1: ${Object.keys(palettes).length} palette pairs, ${ripples.length} ripple palettes, ${surfaces.length - 1} textured surfaces, ${heads.length * eyes.length * 6} specialist defaults`,
);
