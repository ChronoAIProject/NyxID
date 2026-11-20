import { translateApp } from "../../i18n/appTranslation";
import type { AgentIdentity } from "../../components/agentAvatarModel";
import { chatAgentLabel } from "../assistant/assistantAgentContext";
import {
  QUICK_ACCESS_DESTINATIONS,
  QUICK_ACCESS_MAX_ITEMS,
  sameQuickAccessItem,
  type QuickAccessDestination,
  type QuickAccessItem,
} from "../../lib/api/quickAccessSchema";

/** The side menu's name for a destination (its quick-access label). */
export function quickAccessDestinationLabel(id: QuickAccessDestination) {
  switch (id) {
    case "new_chat":
      return translateApp("New chat");
    case "notifications":
      return translateApp("Notifications");
    case "scan_qr":
      return translateApp("Scan QR code");
    case "terminal_login_code":
      return translateApp("Terminal login code");
    case "recent_chats":
      return translateApp("Recent chats");
    case "automations":
      return translateApp("Automations");
    case "machines":
      return translateApp("Machines");
    case "plugins":
      return translateApp("Plugins");
    case "activity":
      return translateApp("Activity");
    case "create_agent":
      return translateApp("Create new agent");
    case "settings":
      return translateApp("Settings");
    case "agents":
      return translateApp("Agents");
    case "crews":
      return translateApp("Crews");
  }
}

/** The fields quick access needs from an agent (the agents list's rows). */
export type QuickAccessAgent = AgentIdentity & {
  readonly kind: "nyxbot" | "specialist";
  readonly status?: "running" | "idle" | "destroyed";
};
/** The fields quick access needs from a crew (the crews list's rows). */
export type QuickAccessCrew = {
  readonly id: string;
  readonly name: string;
  readonly members: readonly AgentIdentity[];
};

/** One entry the rail (and Settings) shows, with its spoken label. */
export type QuickAccessEntry =
  | {
      key: string;
      item: QuickAccessItem;
      kind: "destination";
      id: QuickAccessDestination;
      name: string;
      label: string;
    }
  | {
      key: string;
      item: QuickAccessItem;
      kind: "agent";
      id: string;
      name: string;
      label: string;
      agent: AgentIdentity;
    }
  | {
      key: string;
      item: QuickAccessItem;
      kind: "crew";
      id: string;
      name: string;
      label: string;
      members: readonly AgentIdentity[];
    };

const sameId = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** A pinnable agent: still alive (destroyed agents can't be chatted with). */
export function liveQuickAccessAgents(agents: readonly QuickAccessAgent[] | undefined) {
  return (agents ?? []).filter((agent) => agent.status !== "destroyed");
}

/**
 * The saved items as the rail shows them, in order. Agents and crews come
 * from their lists: while a list is loading, on error, or once the agent is
 * deleted (or destroyed) or the crew gone, the entry is skipped. Notifications
 * follows its feature check, like the side menu's row.
 */
export function resolveQuickAccess(
  items: readonly QuickAccessItem[],
  sources: {
    agents?: readonly QuickAccessAgent[];
    crews?: readonly QuickAccessCrew[];
    notificationsEnabled: boolean;
  },
): QuickAccessEntry[] {
  return items.flatMap((item): QuickAccessEntry[] => {
    const key = `${item.kind}:${item.id.toLowerCase()}`;
    if (item.kind === "destination") {
      if (item.id === "notifications" && !sources.notificationsEnabled)
        return [];
      const name = quickAccessDestinationLabel(item.id);
      return [{ key, item, kind: "destination", id: item.id, name, label: name }];
    }
    if (item.kind === "agent") {
      const agent = liveQuickAccessAgents(sources.agents).find((candidate) =>
        sameId(candidate.id, item.id),
      );
      if (!agent) return [];
      const name = chatAgentLabel(agent);
      return [
        {
          key,
          item,
          kind: "agent",
          id: agent.id,
          name,
          label: translateApp("Chat with {{label}}", { label: name }),
          agent,
        },
      ];
    }
    const crew = sources.crews?.find((candidate) => sameId(candidate.id, item.id));
    if (!crew) return [];
    return [
      {
        key,
        item,
        kind: "crew",
        id: crew.id,
        name: crew.name,
        label: translateApp("Open crew {{name}}", { name: crew.name }),
        members: crew.members,
      },
    ];
  });
}

/** Whether another entry fits (the account holds at most 12). */
export function quickAccessFull(items: readonly QuickAccessItem[]) {
  return items.length >= QUICK_ACCESS_MAX_ITEMS;
}

/** Append an entry unless it is already there or the list is full. */
export function addQuickAccess(
  items: readonly QuickAccessItem[],
  item: QuickAccessItem,
): QuickAccessItem[] {
  if (quickAccessFull(items) || items.some((existing) => sameQuickAccessItem(existing, item)))
    return [...items];
  return [...items, item];
}

export function removeQuickAccess(
  items: readonly QuickAccessItem[],
  index: number,
): QuickAccessItem[] {
  return items.filter((_, position) => position !== index);
}

/** Move the entry at `index` one place up (-1) or down (+1); ends stay put. */
export function moveQuickAccess(
  items: readonly QuickAccessItem[],
  index: number,
  delta: -1 | 1,
): QuickAccessItem[] {
  const target = index + delta;
  if (index < 0 || index >= items.length || target < 0 || target >= items.length)
    return [...items];
  const next = [...items];
  [next[index], next[target]] = [next[target]!, next[index]!];
  return next;
}

/** What can still be added: destinations, live agents and crews not yet in the list. */
export function quickAccessCandidates(
  items: readonly QuickAccessItem[],
  sources: {
    agents?: readonly QuickAccessAgent[];
    crews?: readonly QuickAccessCrew[];
  },
) {
  const has = (item: QuickAccessItem) =>
    items.some((existing) => sameQuickAccessItem(existing, item));
  return {
    destinations: QUICK_ACCESS_DESTINATIONS.filter(
      (id) => !has({ kind: "destination", id }),
    ),
    agents: liveQuickAccessAgents(sources.agents).filter(
      (agent) => !has({ kind: "agent", id: agent.id }),
    ),
    crews: (sources.crews ?? []).filter(
      (crew) => !has({ kind: "crew", id: crew.id }),
    ),
  };
}
