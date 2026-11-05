import {
  CmaSessionContext,
  cmaSessionKey,
  type Observation,
  type ObservedCmaSession,
} from "./cma-session-context";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useState,
  type ReactNode,
} from "react";
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
  const [observed, setObserved] = useState<{
    ownerId: string | undefined;
    sessions: readonly ObservedCmaSession[];
  }>({ ownerId, sessions: [] });
  if (observed.ownerId !== ownerId) {
    setObserved({ ownerId, sessions: [] });
  }
  const publish = useCallback(
    (owner: string, sessions: readonly ObservedCmaSession[]) => {
      setObserved((previous) =>
        previous.ownerId === owner ? { ownerId: owner, sessions } : previous,
      );
    },
    [],
  );
  // Account changes reset the observer, while login routes keep their in-flight
  // completion handlers mounted through profile restoration.
  return (
    <CmaSessionContext
      value={observed.ownerId === ownerId ? observed.sessions : []}
    >
      {ownerId && (
        <OwnedCmaSessions key={ownerId} ownerId={ownerId} onChange={publish} />
      )}
      {children}
    </CmaSessionContext>
  );
}
function OwnedCmaSessions({
  ownerId,
  onChange,
}: {
  readonly ownerId: string;
  readonly onChange: (
    ownerId: string,
    sessions: readonly ObservedCmaSession[],
  ) => void;
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
  useLayoutEffect(() => {
    onChange(
      ownerId,
      sessions.map((session) => ({
        session,
        ...(observations[cmaSessionKey(session)] ?? {
          state: initialCmaSession(session),
          reconnect: () => undefined,
        }),
      })),
    );
  }, [ownerId, sessions, observations, onChange]);
  return sessions.map((session) => (
    <SessionStream
      key={cmaSessionKey(session)}
      ownerId={ownerId}
      session={session}
      publish={publish}
    />
  ));
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
