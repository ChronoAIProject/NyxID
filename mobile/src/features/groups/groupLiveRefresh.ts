import type { QueryKey } from "@tanstack/react-query";
import type { AssistantLiveEvent } from "../../lib/api/assistantLiveTransport";

export const groupKeys = (epoch: number) =>
  ["workspace", "groups", epoch] as const;

export type GroupLiveRefresh = {
  readonly now: readonly QueryKey[];
  readonly lists: readonly QueryKey[];
};

const MAX_SEEN_THREADS = 256;
const AGENTS = ["workspace", "agents"] as const;

/** Identifiers invalidate saved reads; live events never supply message content. */
export function groupLiveRefresh(
  epoch: number,
  event: AssistantLiveEvent,
  seen: Map<string, string>,
): GroupLiveRefresh {
  const root = groupKeys(epoch);
  const list = [...root, "list"] as const;
  const threads = ["workspace", "drawer-chats", epoch] as const;
  switch (event.type) {
    case "group":
      // Personal and organization groups use the same wire event.
      return { now: [[...root, "messages", event.id]], lists: [list] };
    case "conversation": {
      const key = `${epoch}:${event.id}`;
      const signature = `${event.turn_id ?? ""}:${String(event.messages)}`;
      const changed = event.title_changed || seen.get(key) !== signature;
      // Map.set preserves insertion order for known threads: eviction is FIFO.
      seen.set(key, signature);
      while (seen.size > MAX_SEEN_THREADS) {
        const oldest = seen.keys().next();
        if (oldest.done) break;
        seen.delete(oldest.value);
      }
      return {
        now: event.group_id ? [[...root, "messages", event.group_id]] : [],
        lists: changed
          ? event.group_id
            ? [list, AGENTS]
            : [threads, AGENTS]
          : [],
      };
    }
    case "ready":
    case "resync":
      // Reconnection may have missed edits or permission changes.
      seen.clear();
      return { now: [root, AGENTS, threads], lists: [] };
    case "channels":
    case "channel_thread":
      return { now: [], lists: [threads, AGENTS] };
  }
}
