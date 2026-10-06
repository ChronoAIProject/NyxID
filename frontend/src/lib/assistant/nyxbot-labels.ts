import { getProviderBrand } from "@/lib/provider-branding";
import type { AssistantAgent, AssistantAgentKind } from "@/schemas/assistant-nyxagent";
import type { Conversation } from "@/types/assistant";

const PLATFORM_NAMES: Readonly<Record<string, string>> = {
  "telegram-new": "Telegram",
  whatsapp: "WhatsApp",
  x: "X",
  aurinko: "Email",
};

/** Display name of a channel platform id ("telegram" -> "Telegram"). */
export function channelPlatformName(platform: string): string {
  const known = PLATFORM_NAMES[platform] ?? getProviderBrand(platform).label;
  if (known) return known;
  return platform ? platform.charAt(0).toUpperCase() + platform.slice(1) : "Channel";
}

/** A channel chat's name, or what kind of chat it is when unnamed. */
export function channelChatTitle(
  kind: string | null | undefined,
  title: string | null | undefined,
  platform: string,
): string {
  if (title) return title;
  const name = channelPlatformName(platform);
  if (kind === "private") return `Private ${name} chat`;
  if (kind === "channel") return `${name} channel`;
  return kind === "group" ? `${name} group` : `${name} chat`;
}

/** An agent's threads through one channel bot, newest first. */
export interface ChannelThreadGroup {
  /** The channel bot connection, or `platform:<id>` for older threads. */
  readonly key: string;
  readonly platform: string;
  readonly label: string;
  readonly threads: readonly Conversation[];
}

/**
 * Split an agent's threads (newest first) into its own threads and one group
 * per channel bot, ordered by each bot's newest thread.
 */
export function splitChannelThreads(threads: readonly Conversation[]): {
  readonly own: readonly Conversation[];
  readonly bots: readonly ChannelThreadGroup[];
} {
  const own: Conversation[] = [];
  const bots = new Map<string, { platform: string; label?: string; threads: Conversation[] }>();
  for (const thread of threads) {
    const channel = thread.channel;
    if (!channel) {
      own.push(thread);
      continue;
    }
    const key = channel.channel_agent_id ?? `platform:${channel.platform}`;
    const group = bots.get(key) ?? { platform: channel.platform, threads: [] };
    group.label ??= channel.bot_label ?? undefined;
    group.threads.push(thread);
    bots.set(key, group);
  }
  return {
    own,
    bots: [...bots].map(([key, group]) => ({
      key,
      platform: group.platform,
      label: group.label ?? `${channelPlatformName(group.platform)} bot`,
      threads: group.threads,
    })),
  };
}

export const AGENT_STATUS_LABEL = {
  running: "Running",
  idle: "Idle",
  destroyed: "Destroyed",
} as const;

// "NyxID events (authored by NyxID; ...):" -- the parenthetical has changed
// wording over time, so any NyxID events header is stripped.
const EVENT_HEADER = /^NyxID events[^:\n]*:\s*/;

/** The individual notices of a server-authored `event` message. */
export function eventNotices(text: string): string[] {
  const body = text.replace(EVENT_HEADER, "");
  const lines = body
    .split("\n")
    .map((line) => line.replace(/^\s*-\s+/, "").trim())
    .filter(Boolean);
  return lines.length ? lines : [text.trim()];
}

export const AGENT_KIND_LABEL = { nyxbot: "NyxBot", specialist: "Specialist" } as const;

export type EventNoticeKind =
  | "reply"
  | "request"
  | "decision"
  | "approval"
  | "connection"
  | "channel"
  | "message"
  | "notice";
export type EventNoticeTone = "neutral" | "success" | "warning" | "destructive";

/** One NyxID notice, summarised for a compact activity card. */
export interface EventNotice {
  readonly kind: EventNoticeKind;
  readonly tone: EventNoticeTone;
  /** One line: who did what. */
  readonly summary: string;
  /** The rest (a reply excerpt, a reason), shown when the card is expanded. */
  readonly detail?: string;
  /** The specialist the notice is about, when it names one. */
  readonly agent?: string;
}

function firstSentence(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  const end = flat.search(/[.!?](\s|$)/);
  const sentence = end > 0 ? flat.slice(0, end) : flat;
  return sentence.length > 120 ? `${sentence.slice(0, 119)}…` : sentence;
}

/**
 * Classify a server-authored notice (see the backend `assistant_team` and
 * `nyxbot` notifiers) into a compact card. Unknown shapes fall back to their
 * first sentence, so new notices still render.
 */
export function describeEventNotice(text: string): EventNotice {
  const notice = text.trim();
  // The server swaps double quotes in quoted text for single ones.
  let match = /^Specialist (\S+) (replied|was stopped|failed \(([^)]*)\))\. Reply excerpt: "([^"]*)"/.exec(
    notice,
  );
  if (match) {
    const [, agent = "", status = ""] = match;
    const failed = status.startsWith("failed");
    return {
      kind: "reply",
      tone: failed ? "destructive" : status === "replied" ? "neutral" : "warning",
      summary: `${agent} ${failed ? "failed" : status}`,
      detail: match[4]?.trim() || undefined,
      agent,
    };
  }
  match = /^Specialist (\S+) requests (?:service (\S+)|read-only account access) \(request_id [^)]*\)\.\s*([\s\S]*?)(?:\s*Decide with [\s\S]*)?$/.exec(
    notice,
  );
  if (match) {
    const [, agent = "", service] = match;
    return {
      kind: "request",
      tone: "warning",
      summary: service ? `${agent} asks to use ${service}` : `${agent} asks to read your account`,
      detail: match[3]?.trim() || undefined,
      agent,
    };
  }
  match = /^(NyxBot|The user) (allowed|denied) your request for (?:service (\S+)|read-only account access)\.(?:\s*Reason: "([^"]*)")?/.exec(
    notice,
  );
  if (match) {
    const [, by = "", verdict = "", service, reason] = match;
    return {
      kind: "decision",
      tone: verdict === "allowed" ? "success" : "destructive",
      summary: `${by === "NyxBot" ? "NyxBot" : "You"} ${verdict} ${service ?? "read-only account access"}`,
      detail: reason?.trim() || undefined,
    };
  }
  match = /^The user (approved|rejected) the (\S+) approval request \(([\s\S]*?)\)(?: via (\S+))?\./.exec(
    notice,
  );
  if (match) {
    const [, verdict = "", service = "", description, via] = match;
    return {
      kind: "approval",
      tone: verdict === "approved" ? "success" : "destructive",
      summary: `You ${verdict} the ${service} request${via ? ` via ${channelPlatformName(via)}` : ""}`,
      detail: description?.trim() || undefined,
    };
  }
  match = /^The user (finished|declined) connecting (\S+?)(?: \(connect_link_id [^)]*\))?\./.exec(notice);
  if (match) {
    const [, verdict = "", service = ""] = match;
    return {
      kind: "connection",
      tone: verdict === "finished" ? "success" : "neutral",
      summary: `${verdict === "finished" ? "Finished" : "Declined"} connecting ${service}`,
    };
  }
  match = /^The (\S+) channel bot (\S+) (?:the user just created is now linked to (\S+)\.|was created but could not be linked)/.exec(
    notice,
  );
  if (match) {
    const [, platform = "", label = "", agent] = match;
    const name = channelPlatformName(platform);
    return {
      kind: "channel",
      tone: agent ? "success" : "destructive",
      summary: agent
        ? `${name} bot ${label} linked to ${agent}`
        : `${name} bot ${label} could not be linked`,
    };
  }
  match = /^The user verified their (\S+) account on channel bot (\S+);/.exec(notice);
  if (match) {
    const [, platform = "", label = ""] = match;
    return {
      kind: "channel",
      tone: "success",
      summary: `Verified your ${channelPlatformName(platform)} account on ${label}`,
    };
  }
  match = /^The owner sent another (\S+) message while you were working\.[\s\S]*?: "([^"]*)"/.exec(
    notice,
  );
  if (match) {
    return {
      kind: "message",
      tone: "neutral",
      summary: `New ${channelPlatformName(match[1] ?? "")} message`,
      detail: match[2]?.trim() || undefined,
    };
  }
  const summary = firstSentence(notice);
  return {
    kind: "notice",
    tone: "neutral",
    summary,
    detail: summary.length < notice.replace(/\s+/g, " ").trim().length ? notice : undefined,
  };
}

/** "Good morning" / "Good afternoon" / "Good evening" by local hour. */
export function greetingFor(date: Date): string {
  const hour = date.getHours();
  if (hour < 5 || hour >= 18) return "Good evening";
  if (hour < 12) return "Good morning";
  return "Good afternoon";
}

/** "researcher is working…", "researcher and NyxBot are working…", ... */
export function workingLabel(names: readonly string[]): string {
  if (names.length === 1) return `${names[0]!} is working…`;
  if (names.length === 2) return `${names[0]!} and ${names[1]!} are working…`;
  return `${names[0]!}, ${names[1]!} and ${String(names.length - 2)} more are working…`;
}

interface NamedAgent {
  readonly kind: AssistantAgentKind;
  /** The @handle ("NyxBot" for NyxBot). */
  readonly name: string;
  readonly display_name?: string | null;
}

/** How an agent is named on screen: its display name, else its handle. */
export function agentTitle(agent: NamedAgent): string {
  return agent.display_name?.trim() || (agent.kind === "nyxbot" ? "NyxBot" : agent.name);
}

/** "@handle" under a display name; undefined when the title already is the handle. */
export function agentHandle(agent: NamedAgent): string | undefined {
  const handle = agent.kind === "nyxbot" ? "NyxBot" : agent.name;
  return agentTitle(agent) === handle ? undefined : `@${handle}`;
}

/**
 * Conversation and group payloads carry only `{id, name, kind}`; the display
 * name lives on the agent list. Join by id.
 */
export function withDisplayName<T extends { readonly id: string }>(
  ref: T,
  agents: readonly { readonly id: string; readonly display_name: string | null }[] | undefined,
): T & { readonly display_name: string | null } {
  return {
    ...ref,
    display_name: agents?.find((agent) => agent.id === ref.id)?.display_name ?? null,
  };
}

/** A group whose members carry their display names. */
export function groupWithDisplayNames<
  G extends { readonly members: readonly { readonly id: string }[] },
>(
  group: G,
  agents: readonly { readonly id: string; readonly display_name: string | null }[] | undefined,
): Omit<G, "members"> & {
  readonly members: (G["members"][number] & { readonly display_name: string | null })[];
} {
  return { ...group, members: group.members.map((member) => withDisplayName(member, agents)) };
}

/** Owner sections retain server order within each organization. */
export function agentOwnerSections(agents: readonly AssistantAgent[]) {
  const groups = new Map<
    string,
    { id: string; label: string; agents: AssistantAgent[] }
  >();
  for (const agent of agents) {
    const id =
      agent.owner_kind === "org" ? (agent.owner_id ?? "org") : "personal";
    const section = groups.get(id) ?? {
      id,
      label: id === "personal" ? "Personal" : (agent.owner_name ?? id),
      agents: [],
    };
    section.agents.push(agent);
    groups.set(id, section);
  }
  return [...groups.values()].sort(
    (a, b) => Number(b.id === "personal") - Number(a.id === "personal"),
  );
}
