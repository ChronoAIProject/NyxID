import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { User } from "@/types/api";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const group = "catalog:aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const preference = { groups: [], version: 0, updated_at: null };
const pristineFetch = window.fetch;
let dispose: (() => void) | undefined;

beforeEach(() => {
  vi.resetModules();
});
afterEach(() => {
  cleanup();
  dispose?.();
  dispose = undefined;
  window.fetch = pristineFetch;
  vi.doUnmock("@/lib/mock-data");
  vi.restoreAllMocks();
});

async function setup() {
  const moduleGate = deferred<void>();
  const moduleEntered = vi.fn();
  const isMockMode = vi.fn(() => false);
  const getMockResponse = vi.fn();
  // Delay the actual apiClient DEV import, after the hook's initial check.
  vi.doMock("@/lib/mock-data", async () => {
    moduleEntered();
    await moduleGate.promise;
    return { isMockMode, getMockResponse };
  });
  const query = await import("@tanstack/react-query");
  const { useAuthStore: auth } = await import("@/stores/auth-store");
  const { useKeys } = await import("./use-keys");
  const { useServicePreference, useSaveServiceGroupOrder } =
    await import("./use-service-preference");
  const shim = await import("@/components/cli-wizard/client");
  const client = new query.QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  dispose = () => client.clear();
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <query.QueryClientProvider client={client}>
      {children}
    </query.QueryClientProvider>
  );
  const setActor = (id: string | null) =>
    act(() => auth.setState({ user: id ? ({ id } as User) : null }));
  setActor("A");
  const fetches: { actor: string | undefined; path: string; method: string }[] =
    [];
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input);
    const actor = auth.getState().user?.id;
    fetches.push({ actor, path, method: init?.method ?? "GET" });
    return new Response(
      JSON.stringify(
        path.endsWith("/keys")
          ? { keys: [{ id: `${actor}-connection` }] }
          : preference,
      ),
      { headers: { "content-type": "application/json" } },
    );
  });
  window.fetch = fetch;
  return {
    auth,
    client,
    wrapper,
    setActor,
    fetch,
    fetches,
    moduleGate,
    moduleEntered,
    isMockMode,
    getMockResponse,
    useKeys,
    useServicePreference,
    useSaveServiceGroupOrder,
    shim,
  };
}

it.each(["preference", "keys"] as const)(
  "fences the real %s GET after deferred DEV loading and keeps identity caches separate",
  async (kind) => {
    const runtime = await setup();
    const useSubject =
      kind === "keys" ? runtime.useKeys : runtime.useServicePreference;
    const { result } = renderHook(() => useSubject(), {
      wrapper: runtime.wrapper,
    });
    await waitFor(() => expect(runtime.moduleEntered).toHaveBeenCalledOnce());
    runtime.setActor("B");
    runtime.moduleGate.resolve();
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(runtime.fetches).toEqual([
      {
        actor: "B",
        path: kind === "keys" ? "/api/v1/keys" : "/api/v1/service-preferences",
        method: "GET",
      },
    ]);
    expect(runtime.isMockMode).toHaveBeenCalledOnce();
    expect(runtime.getMockResponse).not.toHaveBeenCalled();
    const key = kind === "keys" ? ["keys", "list"] : ["service-preference"];
    expect(runtime.client.getQueryData([...key, "A"])).toBeUndefined();
    expect(runtime.client.getQueryData([...key, "B"])).toEqual(
      kind === "keys" ? [{ id: "B-connection" }] : preference,
    );
  },
);

it.each(["save", "release"] as const)(
  "fences the real %s transport after deferred DEV loading while leaving B usable",
  async (operation) => {
    const runtime = await setup();
    const { result } = renderHook(() => runtime.useSaveServiceGroupOrder(), {
      wrapper: runtime.wrapper,
    });
    let old!: Promise<unknown>;
    act(() => {
      old = (
        operation === "save"
          ? result.current.save(group, { ordered: [], expected_version: 0 })
          : result.current.release(0)
      ).catch((error: unknown) => error);
    });
    await waitFor(() => expect(runtime.moduleEntered).toHaveBeenCalledOnce());
    runtime.setActor("B");
    runtime.moduleGate.resolve();
    expect(await old).toEqual(
      new Error("Account changed before saving agent order"),
    );
    expect(runtime.fetch).not.toHaveBeenCalled();
    expect(runtime.isMockMode).not.toHaveBeenCalled();
    expect(runtime.getMockResponse).not.toHaveBeenCalled();
    await act(async () => {
      await (operation === "save"
        ? result.current.save(group, { ordered: [], expected_version: 0 })
        : result.current.release(0));
    });
    expect(runtime.fetches).toEqual([
      {
        actor: "B",
        path:
          operation === "save"
            ? `/api/v1/service-preferences/groups/${encodeURIComponent(group)}`
            : "/api/v1/service-preferences/hidden",
        method: operation === "save" ? "PUT" : "DELETE",
      },
    ]);
  },
);

it("rejects old Mode A authority after deferred DEV loading before the new shim can proxy it", async () => {
  const runtime = await setup();
  runtime.setActor(null);
  const bootstrap = {
    flow: "api-key-create",
    csrf: "local-A",
    baseUrl: "https://backend.example",
    context: "local",
  } as const;
  runtime.shim.installModeAFetchShim(bootstrap);
  const oldActor = runtime.shim.modeAQueryIdentity();
  const { result, rerender } = renderHook(() => runtime.useKeys(), {
    wrapper: runtime.wrapper,
  });
  await waitFor(() => expect(runtime.moduleEntered).toHaveBeenCalledOnce());
  runtime.shim.installModeAFetchShim({ ...bootstrap, csrf: "local-B" });
  rerender();
  runtime.moduleGate.resolve();
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(runtime.fetch).toHaveBeenCalledOnce();
  const [request, init] = runtime.fetch.mock.calls[0]!;
  expect(String(request)).toContain("/api/proxy/api/v1/keys");
  expect(new Headers(init?.headers).get("x-wizard-csrf")).toBe("local-B");
  expect(
    runtime.client.getQueryData(["keys", "list", oldActor]),
  ).toBeUndefined();
  expect(
    runtime.client.getQueryData([
      "keys",
      "list",
      runtime.shim.modeAQueryIdentity(),
    ]),
  ).toBeDefined();
});

it.each(["preference", "keys"] as const)(
  "ignores a delayed A 401 for %s without clearing B or filling A's cache",
  async (kind) => {
    const runtime = await setup();
    runtime.moduleGate.resolve();
    const response = deferred<Response>();
    runtime.fetch.mockImplementationOnce(() => response.promise);
    const useSubject =
      kind === "keys" ? runtime.useKeys : runtime.useServicePreference;
    const { result } = renderHook(() => useSubject(), {
      wrapper: runtime.wrapper,
    });
    await waitFor(() => expect(runtime.fetch).toHaveBeenCalledOnce());
    runtime.setActor("B");
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    response.resolve(
      new Response(
        JSON.stringify({
          error: "unauthorized",
          error_code: 1001,
          message: "A expired",
        }),
        { status: 401 },
      ),
    );
    const key = kind === "keys" ? ["keys", "list"] : ["service-preference"];
    await waitFor(() =>
      expect(runtime.client.getQueryState([...key, "A"])?.status).toBe("error"),
    );
    expect(runtime.auth.getState().user?.id).toBe("B");
    expect(runtime.client.getQueryData([...key, "A"])).toBeUndefined();
    expect(runtime.client.getQueryData([...key, "B"])).toBeDefined();
    await act(async () => {
      await result.current.refetch();
    });
    expect(runtime.auth.getState().user?.id).toBe("B");
  },
);

it("rejects a delayed successful A response body before it can fill the old cache", async () => {
  const runtime = await setup();
  runtime.moduleGate.resolve();
  const body = deferred<unknown>();
  const parse = vi.fn(() => body.promise);
  runtime.fetch.mockResolvedValueOnce({
    ok: true,
    status: 200,
    json: parse,
  } as unknown as Response);
  const { result } = renderHook(() => runtime.useServicePreference(), {
    wrapper: runtime.wrapper,
  });
  await waitFor(() => expect(parse).toHaveBeenCalledOnce());
  runtime.setActor("B");
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  body.resolve(preference);
  await waitFor(() =>
    expect(
      runtime.client.getQueryState(["service-preference", "A"])?.status,
    ).toBe("error"),
  );
  expect(
    runtime.client.getQueryData(["service-preference", "A"]),
  ).toBeUndefined();
  expect(runtime.client.getQueryData(["service-preference", "B"])).toEqual(
    preference,
  );
});
