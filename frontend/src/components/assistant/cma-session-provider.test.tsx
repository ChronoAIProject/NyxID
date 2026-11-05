import { useLayoutEffect } from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { cmaSession } from "@/lib/assistant/cma-session.test-fixtures";
import { initialCmaSession } from "@/lib/assistant/cma-session-api";
import type {
  CmaSessionReference,
  CmaSessionState,
} from "@/schemas/cma-session";
import { CmaSessionProvider } from "./cma-session-provider";
import { useObservedCmaSessions } from "./cma-session-context";

const api = vi.hoisted(() => ({
  discover: vi.fn(),
  routes: vi.fn(),
  read: vi.fn(),
  watch: vi.fn(),
}));
vi.mock("@/lib/assistant/cma-session-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/assistant/cma-session-api")>()),
  discoverCmaSessions: api.discover,
  resolveCmaRoutes: api.routes,
  readCmaAgent: api.read,
  watchCmaSession: api.watch,
}));

const initial = useAuthStore.getState();
let client: QueryClient;
let snapshots: { owner: string | undefined; titles: string[] }[];
let streams: {
  owner: string;
  signal: AbortSignal;
  emit: (state: CmaSessionState) => void;
}[];
const session: CmaSessionReference = {
  ...cmaSession,
  origin: {
    service_id: cmaSession.connection.id,
    agent_id: cmaSession.agentId,
    conversation_id: "source",
    turn_id: "turn",
    specialist_id: "specialist",
    cma_url: null,
  },
};

function Probe() {
  const owner = useAuthStore((state) => state.user?.id);
  const sessions = useObservedCmaSessions();
  useLayoutEffect(() => {
    snapshots.push({ owner, titles: sessions.map(({ state }) => state.title) });
  });
  return (
    <>
      <input aria-label="Pending login input" />
      {sessions.map(({ session, state }) => (
        <div key={session.agentId}>{state.title}</div>
      ))}
    </>
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  snapshots = [];
  streams = [];
  localStorage.clear();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: false,
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  api.discover.mockResolvedValue([session]);
  api.routes.mockResolvedValue(session.routes);
  api.read.mockImplementation(async (owner: string) => ({
    id: session.agentId,
    title: owner,
    status: "working",
  }));
  api.watch.mockImplementation(
    (
      owner: string,
      _session: CmaSessionReference,
      signal: AbortSignal,
      emit: (state: CmaSessionState) => void,
    ) => {
      streams.push({ owner, signal, emit });
      emit({ ...initialCmaSession(session), title: owner });
      return new Promise(() => undefined);
    },
  );
});

afterEach(() => {
  cleanup();
  client.clear();
  useAuthStore.setState(initial);
});

function setOwner(id: string | null) {
  useAuthStore.getState().setUser(
    id
      ? {
          id,
          email: `${id}@example.com`,
          display_name: id,
          avatar_url: null,
          is_active: true,
          is_admin: false,
          mfa_enabled: true,
          email_verified: true,
          created_at: "2026-01-01T00:00:00Z",
        }
      : null,
  );
}

async function open(path: string) {
  const root = createRootRoute({
    component: () => (
      <CmaSessionProvider>
        <Outlet />
      </CmaSessionProvider>
    ),
  });
  const login = createRoute({
    path: "/login",
    getParentRoute: () => root,
    component: Probe,
  });
  const assistant = createRoute({
    path: "/assistant",
    getParentRoute: () => root,
    component: Probe,
  });
  const router = createRouter({
    routeTree: root.addChildren([login, assistant]),
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
  return screen.findByLabelText("Pending login input");
}

it("keeps the route mounted through sign-in, account replacement, logout and another sign-in", async () => {
  const input = await open("/login");
  await userEvent.type(input, "unfinished verification");
  for (const owner of ["first", "second", null, "second"]) {
    act(() => setOwner(owner));
    expect(screen.getByLabelText("Pending login input")).toBe(input);
    expect(input).toHaveValue("unfinished verification");
  }
  expect(api.discover).not.toHaveBeenCalled();
  expect(api.watch).not.toHaveBeenCalled();
});

it("clears account observations immediately and aborts old streams without remounting the route", async () => {
  const input = await open("/assistant?c=source");
  expect(api.discover).not.toHaveBeenCalled();
  act(() => setOwner("first"));
  await screen.findByText("first");
  await waitFor(() => expect(streams).toHaveLength(1));
  const firstStream = streams[0]!;
  let finishDiscovery!: (sessions: CmaSessionReference[]) => void;
  api.discover.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishDiscovery = resolve;
      }),
  );
  act(() => setOwner("second"));
  expect(screen.queryByText("first")).not.toBeInTheDocument();
  expect(firstStream.signal.aborted).toBe(true);
  expect(screen.getByLabelText("Pending login input")).toBe(input);
  act(() =>
    firstStream.emit({ ...initialCmaSession(session), title: "late first" }),
  );
  expect(screen.queryByText("late first")).not.toBeInTheDocument();
  await act(async () => {
    finishDiscovery([session]);
  });
  await screen.findByText("second");
  await waitFor(() => expect(streams).toHaveLength(2));
  expect(
    snapshots
      .filter(({ owner }) => owner === "second")
      .every(({ titles }) => !titles.includes("first")),
  ).toBe(true);
  act(() => setOwner(null));
  expect(screen.queryByText("second")).not.toBeInTheDocument();
  expect(streams[1]!.signal.aborted).toBe(true);
  expect(snapshots.at(-1)).toEqual({ owner: undefined, titles: [] });
  expect(screen.getByLabelText("Pending login input")).toBe(input);
});
