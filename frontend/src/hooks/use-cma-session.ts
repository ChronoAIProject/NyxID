import { safeCmaUrl } from "@/lib/assistant/cma-url";
import { useEffect, useMemo, useReducer, useState, useRef } from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  type CmaDiscoveryProgress,
  initialCmaSession,
  discoverCmaSessions,
  listPinnedCmaSessions,
  isCmaRetryableError,
  readCmaAgent,
  resolveCmaRoutes,
  watchCmaSession,
} from "@/lib/assistant/cma-session-api";
import type {
  CmaSessionReference,
  CmaSessionState,
} from "@/schemas/cma-session";

export function useActiveCmaSessions(
  ownerId: string,
  conversationId: string | undefined,
  pinned: Readonly<Record<string, boolean>>,
) {
  const client = useQueryClient();
  const [visible, setVisible] = useState<{
    ownerId: string;
    sessions: CmaSessionReference[];
  }>({
    ownerId,
    sessions: [],
  });
  const sourceKey = ["cma-session", ownerId, "origins", conversationId ?? ""];
  const discoveryProgress = useMemo<CmaDiscoveryProgress>(
    () => ({ ownerId, conversationId }),
    [ownerId, conversationId],
  );
  const pinSelection = Object.keys(pinned)
    .filter((key) => pinned[key])
    .sort()
    .join(",");
  const pinKeys = useMemo(
    () => (pinSelection ? pinSelection.split(",") : []),
    [pinSelection],
  );
  const getRoutes = (connectionId: string, bounded: AbortSignal) =>
    client.fetchQuery({
      queryKey: ["cma-session", ownerId, connectionId, "routes"],
      queryFn: ({ signal }) =>
        resolveCmaRoutes(
          ownerId,
          connectionId,
          AbortSignal.any([signal, bounded]),
        ),
      staleTime: 5 * 60_000,
      retry: false,
    });
  const origins = useQuery({
    queryKey: sourceKey,
    queryFn: async ({ signal }) => {
      try {
        return await discoverCmaSessions(
          ownerId,
          AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
          conversationId!,
          getRoutes,
          client.getQueryData<CmaSessionReference[]>(sourceKey),
          discoveryProgress,
        );
      } catch (error) {
        if (!isCmaRetryableError(error)) client.setQueryData(sourceKey, []);
        throw error;
      }
    },
    enabled: Boolean(conversationId),
    staleTime: 15_000,
    refetchInterval: 15_000,
    retry: false,
  });
  const pinQueryKey = useMemo(
    () => ["cma-session", ownerId, "pins", pinKeys] as const,
    [ownerId, pinKeys],
  );
  const scope = JSON.stringify(pinQueryKey);
  const [previousPromotion, setPromotion] = useState({
    scope: "",
    updateCount: 0,
    errorCount: 0,
    sessions: [] as CmaSessionReference[],
  });
  let promotion = previousPromotion;
  if (previousPromotion.scope !== scope) {
    promotion = {
      scope,
      updateCount: client.getQueryState(pinQueryKey)?.dataUpdateCount ?? 0,
      errorCount: client.getQueryState(pinQueryKey)?.errorUpdateCount ?? 0,
      sessions:
        visible.ownerId === ownerId
          ? visible.sessions.filter(
              (session) =>
                session.origin &&
                pinKeys.includes(`${session.connection.id}:${session.agentId}`),
            )
          : [],
    };
    setPromotion(promotion);
  }
  const promotionPending = () => {
    const state = client.getQueryState(pinQueryKey);
    return (
      (state?.dataUpdateCount ?? 0) === promotion.updateCount &&
      ((state?.errorUpdateCount ?? 0) === promotion.errorCount ||
        isCmaRetryableError(state?.error))
    );
  };
  const pins = useQuery({
    queryKey: pinQueryKey,
    // An empty receipt response can equal an older cached [] while still
    // withdrawing a promoted observation. Observe fetch completion as well.
    notifyOnChangeProps: ["data", "error", "isError", "isFetching"],
    queryFn: async ({ signal }) => {
      try {
        return await listPinnedCmaSessions(
          ownerId,
          AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
          pinKeys,
          getRoutes,
          promotionPending()
            ? promotion.sessions
            : (client.getQueryData<CmaSessionReference[]>(pinQueryKey) ?? []),
        );
      } catch (error) {
        if (!isCmaRetryableError(error)) client.setQueryData(pinQueryKey, []);
        throw error;
      }
    },
    // Explicit pinning promotes only an already-visible, server-authorized
    // identity from this account. Refresh immediately, keeping its stream alive
    // through same-document navigation while the pin receipt is pending.
    initialData: () => promotion.sessions,
    initialDataUpdatedAt: 0,
    enabled: pinKeys.length > 0,
    // A pin-selection change must refresh even an existing cached selection.
    // The interval remains 15s; ordinary provider renders do not refetch it.
    staleTime: 0,
    refetchInterval: 15_000,
    retry: false,
  });
  // No placeholder may cross account/source query keys. Transient refreshes
  // retain only the last successful observation for this exact scope.
  const sourceData =
    origins.isError && !isCmaRetryableError(origins.error)
      ? []
      : (origins.data ?? []);
  const pendingPromotion = promotionPending();
  const pinData =
    pins.isError && !isCmaRetryableError(pins.error) && !pendingPromotion
      ? []
      : [
          // An older empty cache entry must not interrupt a newly pinned live
          // observation. Only the next successful receipt read can withdraw it.
          ...(pendingPromotion ? promotion.sessions : []),
          ...(!pendingPromotion &&
          !(pins.isError && !isCmaRetryableError(pins.error))
            ? (pins.data ?? [])
            : []),
        ];
  const candidates = [
    ...new Map(
      [
        ...pinData.filter((s) => pinned[`${s.connection.id}:${s.agentId}`]),
        ...sourceData.filter(
          (s) => s.origin?.conversation_id === conversationId,
        ),
      ].map((s) => [`${s.connection.id}:${s.agentId}`, s]),
    ).values(),
  ].slice(0, 100);
  const active = useQueries({
    queries: candidates.map((session) => ({
      queryKey: [
        "cma-session",
        ownerId,
        session.connection.id,
        session.agentId,
        "active",
      ],
      queryFn: async ({
        signal,
        queryKey,
      }: {
        signal: AbortSignal;
        queryKey: readonly unknown[];
      }) => {
        try {
          const bounded = AbortSignal.any([
            signal,
            AbortSignal.timeout(30_000),
          ]);
          const routes = await getRoutes(session.connection.id, bounded);
          const connection = session.connection;
          const agent = await readCmaAgent(
            ownerId,
            { connection, routes, agentId: session.agentId },
            bounded,
          );
          if (
            !["starting", "working", "waiting_approval", "paused"].includes(
              agent.status,
            )
          )
            return null;
          return {
            connection,
            routes,
            agentId: agent.id,
            callbackUrl:
              agent.callback_url === undefined
                ? undefined
                : (safeCmaUrl(
                    typeof agent.callback_url === "string"
                      ? agent.callback_url
                      : null,
                  ) ?? null),
            title: (agent.owner_title || agent.title || "CMA session").slice(
              0,
              300,
            ),
          };
        } catch (error) {
          if (!isCmaRetryableError(error)) client.setQueryData(queryKey, null);
          throw error;
        }
      },
      staleTime: 5000,
      refetchInterval: (query: { state: { data: unknown } }) =>
        query.state.data === null ? 15_000 : 5000,
      retry: false,
    })),
    // TanStack structurally shares the combined result, keeping the committed
    // visible snapshot stable when detail/receipt values have not changed.
    combine: (sessions) =>
      sessions
        .flatMap((query, i) =>
          query.data && !(query.isError && !isCmaRetryableError(query.error))
            ? [{ ...query.data, origin: candidates[i]!.origin }]
            : [],
        )
        .slice(0, 50),
  });
  if (visible.ownerId !== ownerId || visible.sessions !== active)
    setVisible({ ownerId, sessions: active });
  return active;
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
  const sessionRef = useRef(session);
  useEffect(() => {
    sessionRef.current = session;
  }, [session]);
  const routeKey = JSON.stringify(session.routes);
  useEffect(() => {
    const controller = new AbortController();
    void watchCmaSession(
      ownerId,
      sessionRef.current,
      controller.signal,
      (state) => {
        if (!controller.signal.aborted) setSnapshot({ key, state });
      },
    );
    return () => controller.abort();
  }, [ownerId, key, routeKey, retry]);
  const initial = useMemo(
    () => initialCmaSession({ title: session.title }),
    [session.title],
  );
  const state = snapshot?.key === key ? snapshot.state : initial;
  return { state, reconnect };
}
