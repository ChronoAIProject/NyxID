import { NyxidIcon } from "@/components/brand/nyxid-icon";
import {
  AGENT_AVATAR_TINTS,
  agentInitials,
  agentTintIndex,
} from "@/lib/assistant/nyxbot-avatar";
import { cn } from "@/lib/utils";
import type { AssistantAgentKind } from "@/schemas/assistant-nyxagent";

const SIZES = {
  xs: "h-4 w-4 text-9 tracking-[-0.04em]",
  sm: "h-5 w-5 text-9",
  md: "h-6 w-6 text-9",
  lg: "h-8 w-8 text-11",
} as const;

/**
 * An agent's avatar: NyxBot is the product mark, a specialist is its
 * initials in a circle tinted by its id. Decorative unless `label` is set.
 */
export function AgentAvatar({
  agent,
  size = "md",
  className,
  label,
}: {
  readonly agent: {
    readonly id: string;
    readonly name: string;
    readonly kind: AssistantAgentKind;
    /** Initials come from the display name when there is one. */
    readonly display_name?: string | null;
  };
  readonly size?: keyof typeof SIZES;
  readonly className?: string;
  /** Accessible name; the avatar is hidden from assistive tech without one. */
  readonly label?: string;
}) {
  const a11y = label ? { role: "img", "aria-label": label } : { "aria-hidden": true };
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
        "flex shrink-0 select-none items-center justify-center rounded-full border font-semibold leading-none",
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
}: {
  readonly agents: readonly {
    readonly id: string;
    readonly name: string;
    readonly kind: AssistantAgentKind;
    readonly display_name?: string | null;
  }[];
  readonly max?: number;
  readonly size?: keyof typeof SIZES;
  readonly className?: string;
}) {
  const shown = agents.slice(0, max);
  const hidden = agents.length - shown.length;
  return (
    <span aria-hidden="true" className={cn("flex shrink-0 items-center -space-x-1.5", className)}>
      {shown.map((agent) => (
        <AgentAvatar key={agent.id} agent={agent} size={size} className="ring-2 ring-background" />
      ))}
      {hidden > 0 ? (
        <span
          className={cn(
            "flex items-center justify-center rounded-full border border-hairline bg-muted font-medium text-muted-foreground ring-2 ring-background",
            SIZES[size],
          )}
        >
          +{hidden}
        </span>
      ) : null}
    </span>
  );
}
