import { useMemo } from "react";
import { drawAvatar } from "../../../../shared/agent-avatars/draw";
import {
  AVATAR_BADGE_DISC,
  AVATAR_BADGE_VIEW_BOX,
  avatarPresentation,
  type AvatarBadge,
} from "../../../../shared/agent-avatars/presentation";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { AvatarRenderer } from "@/components/assistant/avatar/avatar-renderer";
import { AvatarSvg } from "@/components/assistant/avatar/avatar-svg";
import {
  resolveAvatar,
  type AvatarConfig,
  type AvatarMood,
} from "@/lib/assistant/agent-avatar-model";
import { useDocumentTheme } from "@/lib/assistant/use-avatar-environment";
import { useAgentAvatarsEnabled } from "@/hooks/use-agent-avatars";
import {
  AGENT_AVATAR_TINTS,
  agentInitials,
  agentTintIndex,
} from "@/lib/assistant/nyxbot-avatar";
import { cn } from "@/lib/utils";
import type { AssistantAgentKind } from "@/schemas/assistant-nyxagent";

const SIZES = {
  xs: "h-4 w-4 type-avatar-initials tracking-[-0.04em]",
  sm: "h-5 w-5 type-avatar-initials",
  md: "h-6 w-6 type-avatar-initials",
  lg: "h-8 w-8 type-avatar-initials-large",
  /** Spans a two-line thread header (28px name line + 16px handle line). */
  xl: "h-11 w-11 type-avatar-initials-header",
  "2xl": "h-14 w-14 type-avatar-initials-header",
  "3xl": "h-18 w-18 type-avatar-initials-header",
  showcase: "h-28 w-28 type-avatar-initials-header",
} as const;

export type AgentAvatarSize = keyof typeof SIZES;

/** Blob avatars: the box only (no initials typography). */
const BLOB_SIZES: Readonly<Record<AgentAvatarSize, string>> = {
  xs: "h-4 w-4",
  sm: "h-5 w-5",
  md: "h-6 w-6",
  lg: "h-8 w-8",
  xl: "h-11 w-11",
  "2xl": "h-14 w-14",
  "3xl": "h-18 w-18",
  showcase: "h-28 w-28",
};
const PIXELS: Readonly<Record<AgentAvatarSize, number>> = {
  xs: 16,
  sm: 20,
  md: 24,
  lg: 32,
  xl: 44,
  "2xl": 56,
  "3xl": 72,
  showcase: 112,
};

const BADGE_PIXELS: Readonly<Record<AgentAvatarSize, number>> = {
  xs: 12,
  sm: 12,
  md: 12,
  lg: 14,
  xl: 16,
  "2xl": 18,
  "3xl": 20,
  showcase: 20,
};
const BADGE_TONES: Readonly<Record<AvatarBadge["tone"], string>> = {
  primary: "text-primary-text",
  warning: "text-warning light:text-amber-800",
  success: "text-success light:text-emerald-700",
  neutral: "text-foreground",
};

function ActivityBadge({
  badge,
  pixels,
  state,
}: {
  readonly badge: AvatarBadge;
  readonly pixels: number;
  readonly state: string;
}) {
  return (
    <svg
      aria-hidden="true"
      focusable="false"
      data-avatar-activity={state}
      data-avatar-activity-tone={badge.tone}
      width={pixels}
      height={pixels}
      viewBox={AVATAR_BADGE_VIEW_BOX}
      className={cn(
        "pointer-events-none absolute -right-0.5 -bottom-0.5 block rounded-full shadow-sm ring-1 ring-foreground/20",
        BADGE_TONES[badge.tone],
      )}
    >
      <circle {...AVATAR_BADGE_DISC} className="fill-background" />
      <path
        d={badge.glyph.d}
        fill={badge.glyph.strokeWidth ? "none" : "currentColor"}
        stroke={badge.glyph.strokeWidth ? "currentColor" : undefined}
        strokeWidth={badge.glyph.strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Below 24px the selected appearance stays still. */
function isSmall(size: AgentAvatarSize): boolean {
  return size === "xs" || size === "sm";
}

export interface AvatarAgent {
  readonly id: string;
  readonly name: string;
  readonly kind: AssistantAgentKind | "unknown";
  /** Initials come from the display name when there is one. */
  readonly display_name?: string | null;
  /** The chosen avatar; absent or unknown derives the default. */
  readonly avatar?: AvatarConfig | null;
}

/**
 * An agent's avatar. With `assistant:avatars` on: the agent's blob (stored or
 * derived), a still at every size unless `animated`, which also needs 24px or
 * more. Off: NyxBot is the product mark, a specialist its tinted initials.
 * Decorative unless `label` is set; the mood is never announced.
 */
export function AgentAvatar({
  agent,
  size = "sm",
  className,
  label,
  mood,
  animated = false,
}: {
  readonly agent: AvatarAgent;
  readonly size?: AgentAvatarSize;
  readonly className?: string;
  /** Accessible name; the avatar is hidden from assistive tech without one. */
  readonly label?: string;
  /** The live mood; stills show its static pose, animated views its motion. */
  readonly mood?: AvatarMood;
  /** Prominent visible avatars share one scheduler and pause off screen. */
  readonly animated?: boolean;
}) {
  useUiTranslation();
  const enabled = useAgentAvatarsEnabled();
  if (enabled) {
    return (
      <BlobAvatar
        agent={agent}
        size={size}
        className={className}
        label={label}
        mood={mood}
        animated={animated}
      />
    );
  }
  return (
    <LegacyAgentAvatar
      agent={agent}
      size={size}
      className={className}
      label={label}
    />
  );
}

function BlobAvatar({
  agent,
  size,
  className,
  label,
  mood,
  animated,
}: {
  readonly agent: AvatarAgent;
  readonly size: AgentAvatarSize;
  readonly className?: string;
  readonly label?: string;
  readonly mood?: AvatarMood;
  readonly animated: boolean;
}) {
  const theme = useDocumentTheme();
  const config = useMemo(
    () =>
      resolveAvatar({
        id: agent.id,
        kind: agent.kind,
        avatar: agent.avatar ?? null,
      }),
    [agent.id, agent.kind, agent.avatar],
  );
  const small = isSmall(size);
  const pixels = PIXELS[size];
  const presentation = avatarPresentation(mood ?? "idle", {
    config,
    size: pixels,
  });
  const box = cn(
    "relative inline-block shrink-0 select-none",
    BLOB_SIZES[size],
    className,
  );
  return (
    <span className={box}>
      {animated ? (
        <AvatarRenderer
          agentId={agent.id}
          kind={agent.kind}
          config={config}
          mood={mood ?? "idle"}
          theme={theme}
          pixels={pixels}
          animated={!small}
          label={label}
          className="block h-full w-full"
        />
      ) : (
        <AvatarStill
          kind={agent.kind}
          config={config}
          mood={mood}
          theme={theme}
          small={small}
          pixels={pixels}
          label={label}
          className="block h-full w-full"
        />
      )}
      {presentation.badge && (presentation.state !== "success" || config.choices.react_success) ? (
        <ActivityBadge
          badge={presentation.badge}
          pixels={BADGE_PIXELS[size]}
          state={presentation.state}
        />
      ) : null}
    </span>
  );
}

/** A plain inline-SVG still (no canvas, no animation loop). */
export function AvatarStill({
  config,
  mood,
  theme,
  small = false,
  pixels,
  kind,
  label,
  className,
}: {
  readonly config: AvatarConfig;
  readonly mood?: AvatarMood;
  readonly theme: "light" | "dark";
  readonly small?: boolean;
  readonly pixels?: number;
  readonly kind?: string;
  readonly label?: string;
  readonly className?: string;
}) {
  const scene = useMemo(
    () =>
      drawAvatar(config, {
        mood,
        theme,
        kind,
        size: small ? 16 : (pixels ?? 44),
        reduced: true,
      }),
    [config, mood, theme, kind, small, pixels],
  );
  return (
    <span className={className}>
      <AvatarSvg
        scene={scene}
        label={label}
        className="block h-full w-full overflow-visible"
      />
    </span>
  );
}

/** Today's avatar, kept unchanged while `assistant:avatars` is off. */
function LegacyAgentAvatar({
  agent,
  size,
  className,
  label,
}: {
  readonly agent: AvatarAgent;
  readonly size: AgentAvatarSize;
  readonly className?: string;
  readonly label?: string;
}) {
  const a11y = label
    ? { role: "img", "aria-label": label }
    : { "aria-hidden": true };
  if (agent.kind === "nyxbot") {
    return (
      <span
        {...a11y}
        className={cn(
          "flex shrink-0 items-center justify-center rounded-full border border-nyx-500/30 bg-nyx-500/10",
          SIZES[size],
          className,
        )}
      >
        <NyxidIcon alt="" className="h-[70%] w-[70%]" />
      </span>
    );
  }
  return (
    <span
      {...a11y}
      className={cn(
        "flex shrink-0 select-none items-center justify-center rounded-full border",
        SIZES[size],
        AGENT_AVATAR_TINTS[agentTintIndex(agent.id)],
        className,
      )}
    >
      {agentInitials(agent.display_name?.trim() || agent.name)}
    </span>
  );
}

/** Overlapping avatars for a group's members, capped with a "+N". */
export function AgentAvatarStack({
  agents,
  max = 3,
  size = "sm",
  className,
  ring = "ring-background",
}: {
  readonly agents: readonly AvatarAgent[];
  readonly max?: number;
  readonly size?: AgentAvatarSize;
  readonly className?: string;
  /** Ring color classes matching the surface behind the stack, so overlaps read as cutouts. */
  readonly ring?: string;
}) {
  useUiTranslation();
  const blobs = useAgentAvatarsEnabled();
  const shown = agents.slice(0, max);
  const hidden = agents.length - shown.length;
  return (
    <span
      aria-hidden="true"
      className={cn("flex shrink-0 items-center -space-x-1.5", className)}
    >
      {shown.map((agent) =>
        blobs ? (
          // Blobs are not round: a muted disc behind each keeps the cutout.
          <span
            key={agent.id}
            className={cn(
              "flex items-center justify-center rounded-full bg-muted ring-2",
              ring,
              SIZES[size],
            )}
          >
            <AgentAvatar
              agent={agent}
              size={size}
              className="h-[86%] w-[86%]"
            />
          </span>
        ) : (
          <AgentAvatar
            key={agent.id}
            agent={agent}
            size={size}
            className={cn("ring-2", ring)}
          />
        ),
      )}
      {hidden > 0 ? (
        <span
          className={cn(
            "flex items-center justify-center rounded-full border border-hairline bg-muted text-muted-foreground ring-2",
            ring,
            SIZES[size],
            size === "xl"
              ? "type-data-total"
              : size === "lg"
                ? "type-secondary-emphasis"
                : "type-group-label",
          )}
        >
          +{hidden}
        </span>
      ) : null}
    </span>
  );
}
