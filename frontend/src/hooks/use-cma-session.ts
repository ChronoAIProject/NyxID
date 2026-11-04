import { useEffect, useReducer, useState } from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  initialCmaSession,
  listActiveCmaSessions,
  listCmaConnections,
  resolveCmaRoutes,
  watchCmaSession,
} from "@/lib/assistant/cma-session-api";
import type {
  CmaSessionReference,
  CmaSessionState,
} from "@/schemas/cma-session";

export function useActiveCmaSessions(ownerId: string) {
  const client = useQueryClient();
  const connections = useQuery({
    queryKey: ["cma-session", ownerId, "connections"],
    queryFn: ({ signal }) =>
      listCmaConnections(
        ownerId,
        AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
      ),
    staleTime: 60_000,
    refetchInterval: 60_000,
    retry: false,
  });
  const sessions = useQueries({
    queries: (connections.data ?? []).map((connection) => ({
      queryKey: ["cma-session", ownerId, connection.id, "active"],
      queryFn: async ({ signal }: { signal: AbortSignal }) => {
        const bounded = AbortSignal.any([signal, AbortSignal.timeout(30_000)]);
        const routes = await client.fetchQuery({
          queryKey: ["cma-session", ownerId, connection.id, "routes"],
          queryFn: ({ signal: routeSignal }) =>
            resolveCmaRoutes(
              ownerId,
              connection.id,
              AbortSignal.any([routeSignal, bounded]),
            ),
          staleTime: 5 * 60_000,
          retry: false,
        });
        return listActiveCmaSessions(ownerId, connection, routes, bounded);
      },
      staleTime: 5000,
      refetchInterval: 5000,
      retry: false,
    })),
  });
  return sessions.flatMap((query) => query.data ?? []);
}

export function useCmaSessionStream(
  ownerId: string,
  session: CmaSessionReference,
) {
  const key = `${ownerId}:${session.connection.id}:${session.agentId}`;
  const [retry, reconnect] = useReducer((value: number) => value + 1, 0);
  const [snapshot, setSnapshot] = useState<{
    key: string;
    state: CmaSessionState;
  }>();
  useEffect(() => {
    const controller = new AbortController();
    void watchCmaSession(ownerId, session, controller.signal, (state) => {
      if (!controller.signal.aborted) setSnapshot({ key, state });
    });
    return () => controller.abort();
  }, [ownerId, session, key, retry]);
  const state =
    snapshot?.key === key ? snapshot.state : initialCmaSession(session);
  return { state, reconnect };
}
