import { act, renderHook, waitFor } from "@testing-library/react";
import {
  MutationCache,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  cmaQueryKeys,
  useCmaSandboxes,
  useCmaSandboxCommand,
} from "./use-nyxbot-cma-sandboxes";

const transport = vi.fn<typeof fetch>();
const clients: QueryClient[] = [];
function setup(
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
) {
  clients.push(client);
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}
beforeEach(() => {
  transport.mockReset();
  vi.stubGlobal("fetch", transport);
  useAuthStore.setState({
    user: {
      id: "owner",
      capabilities: { enabled_features: ["assistant:cma-sandboxes"] },
    } as unknown as User,
  });
});
afterEach(() => {
  clients.forEach((client) => client.clear());
  clients.length = 0;
  vi.unstubAllGlobals();
});

it("uses the owner-scoped management route", async () => {
  transport.mockResolvedValue(
    new Response(
      JSON.stringify({
        sandboxes: [],
        occupied: 0,
        limit: 1,
        conflict: false,
        inventory_complete: true,
      }),
    ),
  );
  const { result } = renderHook(() => useCmaSandboxes(), { wrapper: setup() });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(transport.mock.calls[0]?.[0]).toBe(
    "/api/v1/assistant/nyxagent/sandboxes",
  );
});

it("does not read resources while the feature is disabled", () => {
  useAuthStore.setState({ user: { id: "owner" } as User });
  renderHook(() => useCmaSandboxes(), { wrapper: setup() });
  expect(transport).not.toHaveBeenCalled();
});

it("reads every NyxID inventory page while retaining the server allocation totals", async () => {
  transport.mockResolvedValueOnce(
    new Response(
      JSON.stringify({
        sandboxes: [],
        occupied: 2,
        limit: 1,
        conflict: true,
        inventory_complete: true,
        next_cursor: "page-two",
      }),
    ),
  );
  transport.mockResolvedValueOnce(
    new Response(
      JSON.stringify({
        sandboxes: [],
        occupied: 2,
        limit: 1,
        conflict: true,
        inventory_complete: true,
        next_cursor: null,
      }),
    ),
  );
  const { result } = renderHook(() => useCmaSandboxes(), { wrapper: setup() });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(transport.mock.calls[1]?.[0]).toBe(
    "/api/v1/assistant/nyxagent/sandboxes?cursor=page-two",
  );
  expect(result.current.data).toMatchObject({ occupied: 2, limit: 1 });
});

it("rejects a queued lifecycle action when the signed-in owner changes", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const client = new QueryClient({
    mutationCache: new MutationCache({ onMutate: () => gate }),
  });
  const { result } = renderHook(() => useCmaSandboxCommand(), {
    wrapper: setup(client),
  });
  let pending!: Promise<unknown>;
  act(() => {
    pending = result.current.mutateAsync({
      id: "sandbox",
      generation: 2,
      command: "pause",
      idempotencyKey: "request-one",
    });
  });
  const rejected = expect(pending).rejects.toThrow("account changed");
  act(() => useAuthStore.setState({ user: { id: "other" } as User }));
  await act(async () => {
    release();
    await rejected;
  });
  expect(transport).not.toHaveBeenCalled();
});

it("retains lifecycle identity and invalidates the inventory even on an uncertain error", async () => {
  const client = new QueryClient();
  client.setQueryData(cmaQueryKeys.sandboxes("owner"), { sandboxes: [] });
  transport.mockRejectedValue(new Error("Connection lost"));
  const { result } = renderHook(() => useCmaSandboxCommand(), {
    wrapper: setup(client),
  });
  await act(async () => {
    await expect(
      result.current.mutateAsync({
        id: "sandbox",
        generation: 2,
        command: "pause",
        idempotencyKey: "same-request",
      }),
    ).rejects.toThrow();
  });
  expect(transport.mock.calls[0]?.[0]).toBe(
    "/api/v1/assistant/nyxagent/sandboxes/sandbox/commands",
  );
  expect(JSON.parse(String(transport.mock.calls[0]?.[1]?.body))).toEqual({
    command: "pause",
    generation: 2,
  });
  expect(
    new Headers(transport.mock.calls[0]?.[1]?.headers).get("Idempotency-Key"),
  ).toBe("same-request");
  expect(
    client.getQueryState(cmaQueryKeys.sandboxes("owner"))?.isInvalidated,
  ).toBe(true);
});

it("requests explicit creation recovery without sending a replacement key or body", async () => {
  const client = new QueryClient();
  client.setQueryData(cmaQueryKeys.sandboxes("owner"), { sandboxes: [] });
  transport.mockRejectedValue(new Error("Connection lost"));
  const { result } = renderHook(() => useCmaSandboxCommand(), {
    wrapper: setup(client),
  });
  expect(transport).not.toHaveBeenCalled();
  await act(async () => {
    await expect(
      result.current.mutateAsync({
        id: "local-creation",
        generation: 3,
        command: "recover_creation",
      }),
    ).rejects.toThrow();
  });
  expect(transport).toHaveBeenCalledTimes(1);
  const [url, request] = transport.mock.calls[0]!;
  expect(url).toBe(
    "/api/v1/assistant/nyxagent/sandboxes/local-creation/commands",
  );
  expect(JSON.parse(String(request?.body))).toEqual({
    command: "recover_creation",
    generation: 3,
  });
  expect(new Headers(request?.headers).has("Idempotency-Key")).toBe(false);
  expect(
    client.getQueryState(cmaQueryKeys.sandboxes("owner"))?.isInvalidated,
  ).toBe(true);
});

it.each(["owner", "feature"])(
  "rejects queued creation recovery when %s authority changes",
  async (changed) => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const client = new QueryClient({
      mutationCache: new MutationCache({ onMutate: () => gate }),
    });
    const { result } = renderHook(() => useCmaSandboxCommand(), {
      wrapper: setup(client),
    });
    let pending!: Promise<unknown>;
    act(() => {
      pending = result.current.mutateAsync({
        id: "local-creation",
        generation: 3,
        command: "recover_creation",
      });
    });
    const rejected = expect(pending).rejects.toThrow();
    act(() =>
      useAuthStore.setState({
        user: { id: changed === "owner" ? "other" : "owner" } as User,
      }),
    );
    await act(async () => {
      release();
      await rejected;
    });
    expect(transport).not.toHaveBeenCalled();
  },
);
