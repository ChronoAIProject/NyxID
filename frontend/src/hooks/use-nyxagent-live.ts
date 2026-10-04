import { useEffect } from "react";
import { useQueryClient, type QueryClient, type QueryKey } from "@tanstack/react-query";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { nyxBotGroupKeys } from "@/hooks/use-nyxbot-groups";
import { listenNyxAgentLive, type NyxAgentLiveEvent } from "@/lib/assistant/nyxagent-live";
import { useAuthStore } from "@/stores/auth-store";

/** The open thread or group refreshes this quickly after a change. */
const FAST_FLUSH_MS = 150;
/** Lists (threads, agents, groups, channels) refresh at most this often. */
const LIST_FLUSH_MS = 2000;

export interface NyxAgentLiveRefresh {
  /** The thread or group transcript that changed. */
  readonly now: QueryKey[];
  /** Lists whose rows changed; refreshed at most every two seconds. */
  readonly lists: QueryKey[];
}

/**
 * The queries a live event makes stale. `seen` remembers each thread's
 * running turn and message count, so a tool-activity write refreshes only
 * the open transcript, not the thread and agent lists.
 */
export function nyxAgentLiveRefresh(
  userId: string,
  event: NyxAgentLiveEvent,
  seen: Map<string, string>,
): NyxAgentLiveRefresh {
  const groupList = [...nyxBotGroupKeys.all(userId), "list"];
  switch (event.type) {
    case "conversation": {
      const signature = `${event.turn_id ?? ""}:${String(event.messages)}`;
      const listsChanged = event.title_changed || seen.get(event.id) !== signature;
      seen.set(event.id, signature);
      if (event.group_id) {
        // A hidden group member thread: only its group shows it.
        return {
          now: [nyxBotGroupKeys.messages(userId, event.group_id)],
          lists: listsChanged ? [groupList, nyxBotQueryKeys.agents(userId)] : [],
        };
      }
      return {
        now: [nyxBotQueryKeys.history(userId, event.id)],
        lists: listsChanged
          ? [nyxBotQueryKeys.threads(userId), nyxBotQueryKeys.agents(userId)]
          : [],
      };
    }
    case "group":
      return { now: [nyxBotGroupKeys.messages(userId, event.id)], lists: [groupList] };
    case "channels":
      return {
        now: [],
        lists: [nyxBotQueryKeys.channels(userId), nyxBotQueryKeys.agents(userId)],
      };
    case "ready":
    case "resync":
      // (Re)connected or events were missed: re-read everything.
      seen.clear();
      return { now: [nyxBotQueryKeys.root(userId)], lists: [] };
  }
}

/** One app-wide handler, however many components ask for live updates. */
let active:
  | { client: QueryClient; userId: string; users: number; stop: () => void }
  | undefined;

function start(client: QueryClient, userId: string) {
  const seen = new Map<string, string>();
  const pending = { now: new Map<string, QueryKey>(), lists: new Map<string, QueryKey>() };
  const timers: { now?: ReturnType<typeof setTimeout>; lists?: ReturnType<typeof setTimeout> } =
    {};
  const flush = (which: "now" | "lists") => {
    timers[which] = undefined;
    for (const key of pending[which].values()) {
      // Never cancel a refresh already on its way: bursts would starve it.
      void client.invalidateQueries({ queryKey: key }, { cancelRefetch: false });
    }
    pending[which].clear();
  };
  const stop = listenNyxAgentLive((event) => {
    const refresh = nyxAgentLiveRefresh(userId, event, seen);
    for (const key of refresh.now) pending.now.set(JSON.stringify(key), key);
    for (const key of refresh.lists) pending.lists.set(JSON.stringify(key), key);
    if (pending.now.size) timers.now ??= setTimeout(() => flush("now"), FAST_FLUSH_MS);
    if (pending.lists.size) timers.lists ??= setTimeout(() => flush("lists"), LIST_FLUSH_MS);
  });
  return () => {
    stop();
    if (timers.now) clearTimeout(timers.now);
    if (timers.lists) clearTimeout(timers.lists);
  };
}

/** Refresh NyxBot views when NyxID says something changed. */
export function useNyxAgentLive(enabled: boolean) {
  const queryClient = useQueryClient();
  const userId = useAuthStore((state) => state.user?.id);
  useEffect(() => {
    if (!enabled || !userId) return;
    if (active && (active.client !== queryClient || active.userId !== userId)) {
      active.stop();
      active = undefined;
    }
    active ??= { client: queryClient, userId, users: 0, stop: start(queryClient, userId) };
    const mine = active;
    mine.users += 1;
    return () => {
      mine.users -= 1;
      if (mine.users === 0 && active === mine) {
        mine.stop();
        active = undefined;
      }
    };
  }, [enabled, userId, queryClient]);
}
