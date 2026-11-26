import {
  AVATAR_CATALOGUE,
  processState,
  type AvatarConfig,
  type AvatarMood,
  type ProcessState,
} from "./model";
import { avatarMood, type AvatarMoodInput } from "./lifecycle";
import { quietActivityMotion } from "./quiet-motion";

export interface ProcessFacts {
  readonly surface: "turn" | "voice" | "connection" | "creation" | "editor";
  readonly turn?: AvatarMoodInput;
  readonly voice?: {
    inputAccepting: boolean;
    outputPlaying: boolean;
    outputLevel?: number;
  };
  readonly connection?: {
    status: "connecting" | "ready" | "reconnecting" | "disconnected";
    restoredInSession?: boolean;
  };
  readonly createdInView?: boolean;
  readonly editorChoosing?: boolean;
}
export function resolveProcess(f: ProcessFacts): {
  state: ProcessState;
  connection?: "connecting" | "reconnecting" | "connected";
  level: number;
} {
  const connection =
    f.connection?.status === "connecting"
      ? "connecting"
      : f.connection?.status === "reconnecting"
        ? "reconnecting"
        : f.connection?.status === "ready" && f.connection.restoredInSession
          ? "connected"
          : undefined;
  let state: ProcessState = "idle";
  if (f.surface === "voice")
    state = f.voice?.outputPlaying
      ? "speaking"
      : f.voice?.inputAccepting
        ? "listening"
        : "idle";
  else if (f.surface === "connection") state = connection ?? "idle";
  else if (f.surface === "creation")
    state = f.createdInView ? "created" : "idle";
  else if (f.surface === "editor")
    state = f.editorChoosing ? "choosing" : "idle";
  else state = avatarMood(f.turn ?? {});
  return {
    state,
    connection,
    level: Math.min(1, Math.max(0, f.voice?.outputLevel ?? 0)),
  };
}
export function reactionFor(
  config: AvatarConfig,
  mood: AvatarMood,
  size: number,
  native = false,
  reduced = false,
) {
  const state = processState(mood),
    motion = config.choices.motion ?? "automatic";
  const id =
    config.choices[`react_${state}`] ??
    AVATAR_CATALOGUE.default_reactions[state];
  const entry =
    AVATAR_CATALOGUE.reactions[id as keyof typeof AVATAR_CATALOGUE.reactions] ??
    AVATAR_CATALOGUE.reactions.still;
  const still = reduced || motion === "still" || id === "still";
  const quiet = size < 96 || motion === "quiet" || native;
  const family = config.choices.surface?.startsWith("ascii-")
    ? "character"
    : config.choices.surface?.startsWith("dither-")
      ? "dither"
      : "vector";
  const effective = still
    ? "still"
    : quiet
      ? entry.quiet
      : entry.family !== "vector" && entry.family !== family
        ? entry.vector
        : id;
  return {
    id,
    effective,
    static: entry.static,
    oneShot: entry.oneShot,
    duration: entry.duration,
    quiet,
    still,
    state,
    activityMotion:
      quiet && !still && size >= 24 ? quietActivityMotion(state) : null,
  };
}

export function reactionLabel(id: string): string {
  return id
    .split("-")
    .map((word, index) =>
      index === 0 ? word[0]!.toUpperCase() + word.slice(1) : word,
    )
    .join(" ");
}
