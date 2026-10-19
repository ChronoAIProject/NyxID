import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AccessScopeCard, type AccessScopeState } from "./access-scope-card";
import { installModeAFetchShim, modeAQueryIdentity } from "./client";
import { useAuthStore } from "@/stores/auth-store";

const pristineFetch = window.fetch;
const bootstrap = {
  flow: "api-key-create",
  csrf: "scope-session-A",
  baseUrl: "https://backend.example",
  context: "local",
} as const;
let client: QueryClient;
let requests: { path: string; csrf: string | null }[];

beforeEach(() => {
  useAuthStore.setState({ user: null });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  requests = [];
});
afterEach(() => {
  cleanup();
  client.clear();
  window.fetch = pristineFetch;
  vi.restoreAllMocks();
});

it("uses the real CLI shim and list hook with no dashboard user to select connection and platform scope", async () => {
  const transport = vi.fn(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = new URL(String(input)).pathname;
      requests.push({
        path,
        csrf: new Headers(init?.headers).get("x-wizard-csrf"),
      });
      const data = path.endsWith("/keys")
        ? {
            keys: [
              {
                id: "personal",
                label: "CLI connection",
                slug: "api-slack",
                is_active: true,
                auto_connected: false,
                credential_source: { type: "personal" },
              },
              {
                id: "platform",
                label: "CLI platform",
                slug: "llm-openai",
                is_active: true,
                auto_connected: true,
                credential_source: { type: "personal" },
              },
            ],
          }
        : { nodes: [] };
      return new Response(JSON.stringify(data), {
        headers: { "content-type": "application/json" },
      });
    },
  );
  window.fetch = transport;
  installModeAFetchShim(bootstrap);
  function StandaloneScope() {
    const [scope, setScope] = useState<AccessScopeState>({
      allowAllServices: true,
      allowAllNodes: true,
      selectedServiceIds: new Set(),
      selectedNodeIds: new Set(),
    });
    return (
      <>
        <AccessScopeCard value={scope} onChange={setScope} />
        <output>{[...scope.selectedServiceIds].join(",")}</output>
      </>
    );
  }
  render(
    <QueryClientProvider client={client}>
      <StandaloneScope />
    </QueryClientProvider>,
  );
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("checkbox", { name: "Allow all services" }),
  );
  const connection = await screen.findByRole("checkbox", {
    name: /CLI connection/,
  });
  await user.click(connection);
  expect(connection).toBeChecked();
  await user.click(screen.getByRole("checkbox", { name: "CLI platform" }));
  expect(screen.getByRole("status")).toHaveTextContent("personal,platform");
  expect(useAuthStore.getState().user).toBeNull();
  expect(
    transport.mock.calls.some(([input]) =>
      String(input).includes("/keys?include_tool_bindings=true"),
    ),
  ).toBe(true);
  expect(requests).toEqual(
    expect.arrayContaining([
      { path: "/api/proxy/api/v1/keys", csrf: bootstrap.csrf },
      { path: "/api/proxy/api/v1/nodes", csrf: bootstrap.csrf },
    ]),
  );
  expect(requests.every(({ path }) => path.startsWith("/api/proxy/"))).toBe(
    true,
  );
  expect(requests.some(({ path }) => path.endsWith("/users/me"))).toBe(false);
  expect(
    client.getQueryData(["keys", "list", modeAQueryIdentity(), true]),
  ).toHaveLength(2);
  expect(client.getQueryData(["keys", "list", undefined])).toBeUndefined();
});

it("fences an old CLI query retry after the installed session authority changes", async () => {
  let fail!: (error: unknown) => void;
  const transport = vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
    const csrf = new Headers(init?.headers).get("x-wizard-csrf");
    return csrf === "scope-session-A"
      ? new Promise<Response>((_resolve, reject) => {
          fail = reject;
        })
      : Promise.resolve(
          new Response(JSON.stringify({ keys: [{ id: "B" }] }), {
            headers: { "content-type": "application/json" },
          }),
        );
  });
  // A fresh shim module is needed because it captures the original transport.
  vi.resetModules();
  const shim = await import("./client");
  const query = await import("@tanstack/react-query");
  const { useKeys: scopedKeys } = await import("@/hooks/use-keys");
  const { useAuthStore: auth } = await import("@/stores/auth-store");
  auth.setState({ user: null });
  window.fetch = transport;
  shim.installModeAFetchShim(bootstrap);
  client = new query.QueryClient({
    defaultOptions: { queries: { retry: 1, retryDelay: 0 } },
  });
  const { result, rerender } = renderHook(() => scopedKeys(), {
    wrapper: ({ children }) => (
      <query.QueryClientProvider client={client}>
        {children}
      </query.QueryClientProvider>
    ),
  });
  await waitFor(() => expect(transport).toHaveBeenCalledTimes(1));
  const oldIdentity = shim.modeAQueryIdentity();
  const oldQuery = client
    .getQueryCache()
    .find({ queryKey: ["keys", "list", oldIdentity] })!;
  const observer = new query.QueryObserver(client, {
    ...oldQuery.options,
    queryKey: oldQuery.queryKey,
  });
  const unsubscribe = observer.subscribe(() => {});
  shim.installModeAFetchShim({ ...bootstrap, csrf: "scope-session-B" });
  rerender();
  await waitFor(() => expect(result.current.data).toEqual([{ id: "B" }]));
  await act(async () => fail(new TypeError("disconnected")));
  await waitFor(() => expect(oldQuery.state.status).toBe("error"));
  expect(oldQuery.state.error).toEqual(
    new Error("Account changed before loading connections"),
  );
  expect(transport).toHaveBeenCalledTimes(2);
  expect(client.getQueryData(["keys", "list", oldIdentity])).toBeUndefined();
  expect(
    client.getQueryData(["keys", "list", shim.modeAQueryIdentity()]),
  ).toEqual([{ id: "B" }]);
  unsubscribe();
});
