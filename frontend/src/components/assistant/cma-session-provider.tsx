import {
  CmaSessionContext,
  cmaSessionKey,
  type Observation,
} from "./cma-session-context";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useRouterState } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { useCmaSessionStore } from "@/stores/cma-session-store";
import {
  useActiveCmaSessions,
  useCmaSessionStream,
} from "@/hooks/use-cma-session";
import { initialCmaSession } from "@/lib/assistant/cma-session-api";
import type { CmaSessionReference } from "@/schemas/cma-session";

const EMPTY_PINS: Readonly<Record<string, boolean>> = {};

/** One account-bound observer above all routes; cards and indicators share it. */
export function CmaSessionProvider({
  children,
}: {
  readonly children: ReactNode;
}) {
  const ownerId = useAuthStore((state) => state.user?.id);
  return ownerId ? (
    <OwnedCmaSessions key={ownerId} ownerId={ownerId}>
      {children}
    </OwnedCmaSessions>
  ) : (
    children
  );
}
function OwnedCmaSessions({
  ownerId,
  children,
}: {
  readonly ownerId: string;
  readonly children: ReactNode;
}) {
  const conversation = useRouterState({
    select: (state) => {
      const search = state.location.search as Record<string, unknown>;
      return state.location.pathname === "/assistant" &&
        !search.g &&
        !search.draft &&
        typeof search.c === "string"
        ? search.c
        : undefined;
    },
  });
  const pinned = useCmaSessionStore((state) =>
    state.ownerId === ownerId ? state.pinned : EMPTY_PINS,
  );
  const sessions = useActiveCmaSessions(ownerId, conversation, pinned);
  const [observations, setObservations] = useState<Record<string, Observation>>(
    {},
  );
  const publish = useCallback(
    (key: string, observation: Observation | null) => {
      setObservations((previous) => {
        if (!observation) {
          const next = { ...previous };
          delete next[key];
          return next;
        }
        return { ...previous, [key]: observation };
      });
    },
    [],
  );
  return (
    <CmaSessionContext
      value={sessions.map((session) => ({
        session,
        ...(observations[cmaSessionKey(session)] ?? {
          state: initialCmaSession(session),
          reconnect: () => undefined,
        }),
      }))}
    >
      {sessions.map((session) => (
        <SessionStream
          key={cmaSessionKey(session)}
          ownerId={ownerId}
          session={session}
          publish={publish}
        />
      ))}
      {children}
    </CmaSessionContext>
  );
}
function SessionStream({
  ownerId,
  session,
  publish,
}: {
  readonly ownerId: string;
  readonly session: CmaSessionReference;
  readonly publish: (key: string, observation: Observation | null) => void;
}) {
  const { state, reconnect } = useCmaSessionStream(ownerId, session);
  const key = cmaSessionKey(session);
  useEffect(() => {
    publish(key, { state, reconnect });
  }, [key, state, reconnect, publish]);
  useEffect(() => () => publish(key, null), [key, publish]);
  return null;
}
