/** face-v1: bounded catalogue choices. No renderer or platform dependencies. */
import catalogue from "./catalog.json";
export * from "./lifecycle";
export * from "./process";
export type AvatarTheme = "light" | "dark";
export interface AvatarConfig {
  readonly template: string;
  readonly choices: Readonly<Record<string, string>>;
}
export const AVATAR_TEMPLATE = "face-v1";
export const AVATAR_CATALOGUE = catalogue;
export type AvatarField = keyof typeof catalogue.fields;
export const AVATAR_FIELDS = Object.keys(catalogue.fields) as AvatarField[];
export const AVATAR_PART_SLOTS = ["head", "eyes"] as const;
export const AVATAR_COLOUR_SLOTS = ["palette"] as const;
export const AVATAR_MOODS = [
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
] as const;
export type ProcessState = (typeof AVATAR_MOODS)[number];
/** Older call sites' names normalize to observed states, never persisted. */
export type AvatarMood =
  | ProcessState
  | "talking"
  | "asking"
  | "stressed"
  | "oops"
  | "happy";
const aliases: Partial<Record<AvatarMood, ProcessState>> = {
  talking: "drafting",
  asking: "approval",
  stressed: "retry",
  oops: "failure",
  happy: "success",
};
export function processState(mood: AvatarMood = "idle"): ProcessState {
  return aliases[mood] ?? (mood as ProcessState);
}
export function avatarMoodLabel(mood: AvatarMood): string {
  const state = processState(mood);
  return state === "approval"
    ? "Approval"
    : state[0]!.toUpperCase() + state.slice(1);
}
export function avatarChoices(field: AvatarField, kind?: string): string[] {
  return Object.entries(catalogue.fields[field])
    .filter(
      ([, v]) =>
        !("kinds" in v) || !kind || (v.kinds as string[]).includes(kind),
    )
    .map(([id]) => id);
}
export function avatarChoiceLabel(field: AvatarField, id: string): string {
  return (
    (catalogue.fields[field] as Record<string, { label: string }>)[id]?.label ??
    id
  );
}
export function identifier(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^[a-z][a-z0-9_-]*$/.test(value) &&
    value.length <= AVATAR_CATALOGUE.max_identifier
  );
}
/** Tolerant reads preserve well-formed future settings; no interpretation as code. */
export function readableAvatar(value: unknown): value is AvatarConfig {
  if (!value || typeof value !== "object") return false;
  const v = value as Partial<AvatarConfig>;
  return (
    identifier(v.template) &&
    !!v.choices &&
    typeof v.choices === "object" &&
    !Array.isArray(v.choices) &&
    Object.keys(v.choices).length <= AVATAR_CATALOGUE.max_choices &&
    Object.entries(v.choices).every(
      ([k, id]) => identifier(k) && identifier(id),
    )
  );
}
export function isValidAvatar(value: unknown): value is AvatarConfig {
  if (!readableAvatar(value) || value.template !== AVATAR_TEMPLATE)
    return false;
  return (
    Object.entries(value.choices).every(
      ([key, id]) =>
        AVATAR_FIELDS.includes(key as AvatarField) &&
        avatarChoices(key as AvatarField).includes(id),
    ) &&
    !(
      value.choices.treatment === catalogue.compatibility.stroke_treatment &&
      value.choices.surface &&
      value.choices.surface !== catalogue.compatibility.stroke_surface
    )
  );
}
export function sameAvatar(a: AvatarConfig, b: AvatarConfig): boolean {
  const keys = Object.keys(a.choices);
  return (
    a.template === b.template &&
    keys.length === Object.keys(b.choices).length &&
    keys.every((k) => a.choices[k] === b.choices[k])
  );
}
export function hashId(id: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i++)
    h = Math.imul(h ^ id.charCodeAt(i), 0x01000193);
  return h >>> 0;
}
export const NYXBOT_AVATAR: AvatarConfig = {
  template: AVATAR_TEMPLATE,
  choices: { ...catalogue.defaults },
};
const HEADS = ["block", "tall", "wide", "round"] as const;
const EYES = ["dots", "tall", "calm", "happy", "wink", "visor"] as const;
const PALETTES = [
  "core-violet",
  "core-ocean",
  "core-mint",
  "core-ember",
  "core-indigo",
  "core-graphite",
] as const;
export function defaultAvatar(id: string, kind: string): AvatarConfig {
  if (kind === "nyxbot")
    return { template: AVATAR_TEMPLATE, choices: { ...catalogue.defaults } };
  const n = hashId(id);
  return {
    template: AVATAR_TEMPLATE,
    choices: {
      ...catalogue.defaults,
      treatment: "tint",
      background: "treatment",
      frame: "circle",
      head: HEADS[Math.floor(n / 6) % 4]!,
      eyes: EYES[Math.floor(n / 24) % 6]!,
      palette: PALETTES[n % 6]!,
    },
  };
}
export function resolveAvatar(agent: {
  id: string;
  kind: string;
  avatar?: AvatarConfig | null;
}): AvatarConfig {
  const fallback = defaultAvatar(agent.id, agent.kind);
  if (!isValidAvatar(agent.avatar)) return fallback;
  const choices = { ...fallback.choices, ...agent.avatar.choices };
  // Identity comes from the server-known kind, including malformed historical rows.
  if (agent.kind !== "nyxbot" && choices.head === "logo")
    choices.head = fallback.choices.head!;
  if (agent.kind !== "nyxbot" && choices.palette === "core-nyx")
    choices.palette = fallback.choices.palette!;
  return { template: AVATAR_TEMPLATE, choices };
}
export function chooseAvatar(
  config: AvatarConfig,
  field: AvatarField,
  id: string,
): AvatarConfig {
  const choices = { ...config.choices, [field]: id };
  return { template: AVATAR_TEMPLATE, choices };
}
export function randomAvatar(
  random: () => number = Math.random,
  kind = "specialist",
): AvatarConfig {
  let c = defaultAvatar("", kind);
  for (const field of [
    "head",
    "eyes",
    "palette",
    "treatment",
    "surface",
    "ripple_palette",
  ] as AvatarField[]) {
    const ids = avatarChoices(field, kind).filter((id) =>
      isValidAvatar(chooseAvatar(c, field, id)),
    );
    c = chooseAvatar(
      c,
      field,
      ids[
        Math.min(ids.length - 1, Math.max(0, Math.floor(random() * ids.length)))
      ]!,
    );
  }
  return c;
}

/** Compare stored envelopes without resolving defaults or depending on key order. */
export function sameStoredAvatar(
  a: AvatarConfig | null | undefined,
  b: AvatarConfig | null | undefined,
): boolean {
  if (!a || !b) return (a ?? null) === (b ?? null);
  return sameAvatar(a, b);
}

export function isValidAvatarForKind(
  value: unknown,
  kind: string,
): value is AvatarConfig {
  return (
    isValidAvatar(value) &&
    Object.entries(value.choices).every(([field, id]) =>
      avatarChoices(field as AvatarField, kind).includes(id),
    )
  );
}
