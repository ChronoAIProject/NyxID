import { useNavigate, useRouterState } from "@tanstack/react-router";

declare module "@tanstack/react-router" {
  interface HistoryState {
    agentDetailsOrigin?: Record<string, unknown>;
  }
}

/** Previously local sheet buttons must not add browser history entries. */
export function useAgentDetailsNavigation() {
  const navigate = useNavigate();
  const { search, origin } = useRouterState({
    select: state => ({ search: state.location.search, origin: state.location.state.agentDetailsOrigin }),
  });
  return {
    open: (id: string) => void navigate({
      to: "/assistant",
      search: previous => ({ ...previous, agent: id, details: "overview", proposal: undefined, connection: undefined }),
      state: previous => ({ ...previous, agentDetailsOrigin: origin ?? search }),
      replace: true,
      resetScroll: false,
    }),
    close: (linkedClose?: () => void) => {
      if (!origin && linkedClose) return linkedClose();
      void navigate({
        to: "/assistant",
        search: previous => origin ?? { ...previous, details: undefined, proposal: undefined, connection: undefined },
        state: previous => ({ ...previous, agentDetailsOrigin: undefined }),
        replace: Boolean(origin),
        resetScroll: false,
      });
    },
  };
}
