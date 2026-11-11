import type { QueryClient, QueryKey } from "@tanstack/react-query";
import type {
  AssistantLiveEvent,
  AssistantLiveStatus,
} from "../../lib/api/assistantLiveTransport";
import { groupKeys, groupLiveRefresh } from "./groupLiveRefresh";

export type GroupLiveRunOptions = {
  signal: AbortSignal;
  onStatus: (status: AssistantLiveStatus) => void;
  onEvent: (event: AssistantLiveEvent) => void;
};

type Lane = "now" | "lists";
type Timer = { handle: unknown };
type Session = {
  client: QueryClient;
  epoch: number;
  controller: AbortController;
  consumers: Set<() => boolean>;
  status: AssistantLiveStatus;
  started: boolean;
  seen: Map<string, string>;
  pending: Record<Lane, Map<string, QueryKey>>;
  timers: Partial<Record<Lane, Timer>>;
};

const DELAY: Record<Lane, number> = { now: 150, lists: 2000 };
const MAX_PENDING_KEYS = 256;

/** Share one caller-owned stream without importing native lifecycle APIs. */
export function createGroupLiveSession({
  run,
  available,
  schedule = (callback, milliseconds) => setTimeout(callback, milliseconds),
  cancel = (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
}: {
  run: (options: GroupLiveRunOptions) => Promise<void>;
  available: (epoch: number) => boolean;
  schedule?: (callback: () => void, milliseconds: number) => unknown;
  cancel?: (handle: unknown) => void;
}) {
  let active: Session | undefined;
  const listeners = new Set<() => void>();
  const eventListeners = new Set<{
    client: QueryClient;
    epoch: number;
    receive: (event: AssistantLiveEvent) => void;
  }>();
  const notify = () => {
    for (const listener of [...listeners]) listener();
  };
  const usable = (session: Session) =>
    active === session &&
    !session.controller.signal.aborted &&
    available(session.epoch) &&
    [...session.consumers].some((consumer) => consumer());

  function setStatus(session: Session, status: AssistantLiveStatus) {
    if (!usable(session) || session.status === status) return;
    session.status = status;
    notify();
  }

  function stop(session: Session) {
    if (active === session) active = undefined;
    // Clear ownership first: abort may synchronously deliver stale callbacks.
    session.controller.abort();
    for (const timer of Object.values(session.timers)) cancel(timer.handle);
    session.timers = {};
    session.pending.now.clear();
    session.pending.lists.clear();
    session.seen.clear();
    session.consumers.clear();
    session.status = "idle";
    notify();
  }

  function flush(session: Session, lane: Lane) {
    const keys = [...session.pending[lane].values()];
    session.pending[lane].clear();
    for (const queryKey of keys) {
      if (!usable(session)) return;
      // Bursts must not cancel a read that is already making progress.
      void session.client
        .invalidateQueries({ queryKey }, { cancelRefetch: false })
        .catch(() => undefined);
    }
  }

  function queue(session: Session, lane: Lane, keys: readonly QueryKey[]) {
    if (!usable(session)) return;
    const pending = session.pending[lane];
    const root = groupKeys(session.epoch);
    const rootId = JSON.stringify(root);
    const isGroupKey = (key: QueryKey) =>
      root.every((part, index) => key[index] === part);
    for (const key of keys) {
      if (isGroupKey(key) && pending.has(rootId)) continue;
      if (isGroupKey(key) && key.length === root.length) {
        for (const [id, previous] of pending)
          if (isGroupKey(previous)) pending.delete(id);
      }
      pending.set(JSON.stringify(key), key);
      if (pending.size > MAX_PENDING_KEYS) {
        // One group-root invalidation covers every discarded group identifier.
        for (const [id, previous] of pending)
          if (isGroupKey(previous)) pending.delete(id);
        pending.set(rootId, root);
      }
    }
    if (!pending.size || session.timers[lane]) return;
    const timer: Timer = { handle: undefined };
    session.timers[lane] = timer;
    timer.handle = schedule(() => {
      if (session.timers[lane] !== timer) return;
      delete session.timers[lane];
      flush(session, lane);
    }, DELAY[lane]);
  }

  function start(session: Session) {
    if (session.started || !usable(session)) return;
    session.started = true;
    setStatus(session, "connecting");
    const failed = () => setStatus(session, "connecting");
    try {
      void run({
        signal: session.controller.signal,
        onStatus: (status) => setStatus(session, status),
        onEvent: (event) => {
          if (!usable(session)) return;
          for (const listener of eventListeners) {
            if (
              listener.client === session.client &&
              listener.epoch === session.epoch
            )
              listener.receive(event);
          }
          const refresh = groupLiveRefresh(session.epoch, event, session.seen);
          queue(session, "now", refresh.now);
          queue(session, "lists", refresh.lists);
        },
      }).catch(failed);
    } catch {
      failed();
    }
  }

  return {
    acquire(
      client: QueryClient,
      epoch: number,
      consumerAvailable: () => boolean,
    ) {
      if (active && (active.client !== client || active.epoch !== epoch))
        stop(active);
      const session: Session = active ?? {
        client,
        epoch,
        controller: new AbortController(),
        consumers: new Set(),
        status: "idle",
        started: false,
        seen: new Map(),
        pending: { now: new Map(), lists: new Map() },
        timers: {},
      };
      // Each acquisition owns a distinct entry, even with the same callback.
      const consumer = () => consumerAvailable();
      session.consumers.add(consumer);
      active = session;
      start(session);
      let released = false;
      return () => {
        if (released) return;
        released = true;
        session.consumers.delete(consumer);
        if (active === session && !session.consumers.size) stop(session);
      };
    },
    subscribeEvents(
      client: QueryClient,
      epoch: number,
      receive: (event: AssistantLiveEvent) => void,
    ) {
      const listener = { client, epoch, receive };
      eventListeners.add(listener);
      return () => {
        eventListeners.delete(listener);
      };
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getStatus(client: QueryClient, epoch: number): AssistantLiveStatus {
      return active?.client === client && active.epoch === epoch
        ? active.status
        : "idle";
    },
  };
}
