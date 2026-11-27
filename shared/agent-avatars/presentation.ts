import type { RosterActivityState } from "./activity";
import type { AvatarConfig, AvatarMood, ProcessState } from "./model";
import { reactionFor } from "./process";

export const AVATAR_BADGE_VIEW_BOX = "0 0 24 24";
export const AVATAR_BADGE_DISC = { cx: 12, cy: 12, r: 12 } as const;

export interface AvatarBadge {
  readonly tone: "primary" | "warning" | "success" | "neutral";
  /** Centered in a 24-unit disc. Stroked paths use rounded caps and joins. */
  readonly glyph: { readonly d: string; readonly strokeWidth?: number };
}

const BADGES: Readonly<
  Record<RosterActivityState | "success" | "failure", AvatarBadge>
> = {
  thinking: {
    tone: "primary",
    glyph: {
      d: "M8.1 12a2.3 2.3 0 1 1-4.6 0 2.3 2.3 0 1 1 4.6 0Zm6.2 0a2.3 2.3 0 1 1-4.6 0 2.3 2.3 0 1 1 4.6 0Zm6.2 0a2.3 2.3 0 1 1-4.6 0 2.3 2.3 0 1 1 4.6 0Z",
    },
  },
  working: {
    tone: "primary",
    glyph: { d: "M13.8 4 5.5 13.3h5.2L10 20l8.5-9.8H13L13.8 4Z" },
  },
  drafting: {
    tone: "primary",
    glyph: { d: "M5.5 7.5h13m-13 4.5h13m-13 4.5h8", strokeWidth: 2.5 },
  },
  waiting: {
    tone: "neutral",
    glyph: {
      d: "M5 4.5h14v2.8h-2L12 12 7 7.3H5ZM5 16.7h2l5-4.7 5 4.7h2v2.8H5Z",
    },
  },
  approval: {
    tone: "warning",
    glyph: { d: "M12 5.7v7.1m0 5.2h.01", strokeWidth: 3.5 },
  },
  retry: {
    tone: "neutral",
    glyph: {
      d: "M18.6 9.5A6.9 6.9 0 1 0 17.8 16.1M19.1 4.5v6h-6",
      strokeWidth: 2.7,
    },
  },
  stopping: {
    tone: "neutral",
    glyph: {
      d: "M7 5h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2Z",
    },
  },
  success: {
    tone: "success",
    glyph: { d: "m5.5 12 4.1 4.1 8.9-8.9", strokeWidth: 2.8 },
  },
  failure: {
    tone: "neutral",
    glyph: { d: "m6 6 12 12m0-12L6 18", strokeWidth: 2.8 },
  },
};

export function avatarBadge(state: ProcessState): AvatarBadge | null {
  return Object.hasOwn(BADGES, state)
    ? BADGES[state as keyof typeof BADGES]
    : null;
}

interface PresentationOptions {
  readonly config: AvatarConfig;
  readonly size: number;
  readonly reduced?: boolean;
  readonly native?: boolean;
}

export function avatarPresentation(
  mood: AvatarMood,
  { config, size, reduced, native }: PresentationOptions,
) {
  const reaction = reactionFor(config, mood, size, native, reduced);
  const mode =
    reaction.still || size < 24 || reaction.activityMotion?.period === null
      ? "static"
      : reaction.quiet
        ? "quiet"
        : "rich";
  return {
    state: reaction.state,
    badge: avatarBadge(reaction.state),
    motion: {
      mode,
      effect: mode === "static" ? "still" : reaction.effective,
      signature: reaction.activityMotion,
    },
    oneShot: reaction.oneShot,
    staticFrame: reaction.static,
  } as const;
}
