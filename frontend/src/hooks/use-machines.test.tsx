import {
  MutationCache,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  remoteMachineFixture,
  remoteMachineOperationFixture,
  remoteMachinePaidQuoteFixture,
} from "@/test/remote-machine-fixture";
import {
  readRemoteMachineJobOutput,
  useCreateRemoteMachine,
  useRemoteMachineQuote,
  useRemoteMachineEntitlement,
  useRemoteMachineAction,
  useRemoteMachineJobs,
  useRemoteMachineEvents,
  useRemoteMachineRuntimeBilling,
  useRemoteMachines,
  useMachineSettings,
  useRemoteMachineBillingRuns,
  useRemoteMachineConfirmations,
  useDecideRemoteMachineConfirmation,
} from "./use-machines";

const transport = vi.fn<typeof fetch>();
const createBody = {
  name: "research-machine",
  capabilities: ["shell", "files", "browser", "computer"] as (
    | "shell"
    | "files"
    | "browser"
    | "computer"
  )[],
  specialist_id: "specialist-1",
  free_slot: true,
  profile_version: "remote-machine" as const,
  policy_version: "remote-machine-mvp" as const,
  data_policy_version: "remote-machine-data" as const,
  acknowledge_shared_environment: true,
};
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
function wrapper(client: QueryClient) {
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}
function makeClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

beforeEach(() => {
  transport.mockReset();
  vi.stubGlobal("fetch", transport);
  useAuthStore.setState({
    user: {
      id: "owner-one",
      capabilities: { enabled_features: ["experimental:remote-machines"] },
    } as unknown as User,
    isAuthenticated: true,
    isLoading: false,
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Remote Machine owner HTTP hooks", () => {
  it("strips secret-like additions before storing owner-scoped query data", async () => {
    transport.mockResolvedValueOnce(
      json({
        machines: [
          { ...remoteMachineFixture, bootstrap_token: "fixture-secret" },
        ],
      }),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(() => useRemoteMachines(), {
      wrapper: wrapper(client),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.[0]).toEqual(remoteMachineFixture);
    expect(
      JSON.stringify(client.getQueryData(["remote-machines", "owner-one"])),
    ).not.toContain("fixture-secret");
    expect(transport.mock.calls[0]?.[0]).toBe("/api/v1/remote-machines");
    unmount();
    client.clear();
  });

  it("hides previously authorized machine data when a live read is denied", async () => {
    transport.mockResolvedValueOnce(
      json(
        {
          error: "forbidden",
          error_code: 1002,
          message: "No longer authorized",
        },
        403,
      ),
    );
    const client = makeClient();
    client.setQueryData(
      ["remote-machines", "owner-one"],
      [remoteMachineFixture],
    );
    const { result, unmount } = renderHook(() => useRemoteMachines(), {
      wrapper: wrapper(client),
    });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toBeUndefined();
    unmount();
    client.clear();
  });

  it("keeps mutation callbacks stable for effects while binding them to the owner", () => {
    const client = makeClient();
    const { result, rerender } = renderHook(() => useCreateRemoteMachine(), {
      wrapper: wrapper(client),
    });
    const mutate = result.current.mutate;
    const mutateAsync = result.current.mutateAsync;
    rerender();
    expect(result.current.mutate).toBe(mutate);
    expect(result.current.mutateAsync).toBe(mutateAsync);
    act(() => useAuthStore.setState({ user: { id: "owner-two" } as User }));
    expect(result.current.mutate).not.toBe(mutate);
    client.clear();
  });

  it("preserves the reviewed body and Idempotency-Key across an unknown-result retry", async () => {
    transport.mockRejectedValueOnce(new TypeError("connection lost"));
    transport.mockResolvedValueOnce(
      json(
        {
          machine: remoteMachineFixture,
          operation: remoteMachineOperationFixture,
          replayed: true,
        },
        202,
      ),
    );
    const client = makeClient();
    const { result } = renderHook(() => useCreateRemoteMachine(), {
      wrapper: wrapper(client),
    });
    const request = { body: createBody, idempotencyKey: "stable-intent" };
    await act(async () => {
      await expect(result.current.mutateAsync(request)).rejects.toThrow(
        "connection lost",
      );
    });
    await act(async () => {
      await result.current.mutateAsync(request);
    });
    expect(transport).toHaveBeenCalledTimes(2);
    for (const [, config] of transport.mock.calls) {
      expect(config?.headers).toMatchObject({
        "Idempotency-Key": "stable-intent",
      });
      expect(JSON.parse(config?.body as string)).toEqual(createBody);
    }
    expect(result.current.data?.replayed).toBe(true);
    client.clear();
  });

  it("sends DELETE policy acknowledgement in a body and never auto-retries a revision conflict", async () => {
    transport.mockResolvedValueOnce(
      json(
        {
          error: "conflict",
          error_code: 1009,
          message: "Machine changed; reload",
        },
        409,
      ),
    );
    const client = makeClient();
    const { result } = renderHook(() => useRemoteMachineAction(), {
      wrapper: wrapper(client),
    });
    const body = {
      action: "delete" as const,
      expected_revision: 7,
      data_policy_version: "remote-machine-data" as const,
      acknowledge_data_loss: true,
    };
    await act(async () => {
      await expect(
        result.current.mutateAsync({
          machineId: "machine-1",
          idempotencyKey: "delete-intent",
          body,
        }),
      ).rejects.toThrow("reload");
    });
    expect(transport).toHaveBeenCalledTimes(1);
    expect(transport.mock.calls[0]?.[0]).toBe(
      "/api/v1/remote-machines/machine-1",
    );
    expect(transport.mock.calls[0]?.[1]?.method).toBe("DELETE");
    expect(JSON.parse(transport.mock.calls[0]?.[1]?.body as string)).toEqual(
      body,
    );
    client.clear();
  });

  it("captures the owner before deferred mutation middleware and rejects account switches", async () => {
    let resume!: () => void;
    const gate = new Promise<void>((resolve) => {
      resume = resolve;
    });
    const client = new QueryClient({
      mutationCache: new MutationCache({ onMutate: () => gate }),
    });
    const { result } = renderHook(() => useCreateRemoteMachine(), {
      wrapper: wrapper(client),
    });
    let request!: Promise<unknown>;
    act(() => {
      request = result.current.mutateAsync({
        body: createBody,
        idempotencyKey: "old-account-intent",
      });
    });
    const rejected = expect(request).rejects.toThrow("account changed");
    act(() => {
      useAuthStore.setState({ user: { id: "owner-two" } as User });
    });
    await act(async () => {
      resume();
      await rejected;
    });
    expect(transport).not.toHaveBeenCalled();
    expect(
      client.getQueryData(["remote-machines", "owner-two", "machine-1"]),
    ).toBeUndefined();
    client.clear();
  });

  it("keeps owner confirmation policy human requests bound to the original account", async () => {
    let resume!: () => void;
    const gate = new Promise<void>((resolve) => {
      resume = resolve;
    });
    const client = new QueryClient({
      mutationCache: new MutationCache({ onMutate: () => gate }),
    });
    const { result } = renderHook(() => useMachineSettings("machine-1"), {
      wrapper: wrapper(client),
    });
    let request!: Promise<unknown>;
    act(() => {
      request = result.current.mutateAsync({
        machine_confirm: "all",
        allow_single_user_saved_logins: false,
        acknowledge_single_user_risk: false,
      });
    });
    const rejected = expect(request).rejects.toThrow("account changed");
    act(() => {
      useAuthStore.setState({ user: { id: "owner-two" } as User });
    });
    await act(async () => {
      resume();
      await rejected;
    });
    expect(transport).not.toHaveBeenCalled();
    client.clear();
  });

  it("does not turn missing runtime/job endpoints into free balances or empty history", async () => {
    transport.mockImplementation(async () =>
      json({ error: "not_found", error_code: 1003, message: "Not found" }, 404),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(
      () => ({
        jobs: useRemoteMachineJobs("machine-1", {
          status: "running",
        }),
        billing: useRemoteMachineRuntimeBilling("machine-1"),
      }),
      { wrapper: wrapper(client) },
    );
    await waitFor(() => {
      expect(result.current.jobs.isError).toBe(true);
      expect(result.current.billing.isError).toBe(true);
    });
    expect(result.current.jobs.data).toBeUndefined();
    expect(result.current.billing.data).toBeUndefined();
    expect(transport).toHaveBeenCalledTimes(2);
    expect(transport.mock.calls.map(([url]) => String(url))).toContain(
      "/api/v1/remote-machines/machine-1/jobs?limit=25&state=running",
    );
    unmount();
    client.clear();
  });

  it("loads a bounded rate-history page and binds its cursor to the owner", async () => {
    transport.mockResolvedValueOnce(
      json({
        runs: [
          {
            run_id: "old-run",
            rate_version: "old-rate",
            accepted_credits_per_second: "0.000040",
            state: "closed",
            created_at: "2026-10-14T01:00:00Z",
            updated_at: "2026-10-14T01:00:00Z",
          },
        ],
        next_offset: 50,
      }),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(
      () => useRemoteMachineBillingRuns("machine-1", "25"),
      { wrapper: wrapper(client) },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(transport.mock.calls[0]?.[0]).toBe(
      "/api/v1/remote-machines/machine-1/billing/runs?limit=25&offset=25",
    );
    expect(result.current.data?.runs[0]?.accepted_credits_per_second).toBe(
      "0.000040",
    );
    expect(result.current.data?.next_cursor).toBe("50");
    expect(
      client.getQueryData([
        "remote-machines",
        "owner-two",
        "machine-1",
        "billing",
        "runs",
        "25",
      ]),
    ).toBeUndefined();
    unmount();
    client.clear();
  });

  it("normalizes actual history without inventing output or terminal details", async () => {
    transport.mockResolvedValueOnce(
      json({
        jobs: [
          {
            id: "job-one",
            job_id: "job-one",
            node_id: "machine-1",
            runtime_generation: 1,
            state: "finished",
            binding: { kind: "assistant", legacy: true },
            services: [],
            created_at: "2026-10-14T01:00:00Z",
            expires_at: "2026-10-14T01:02:00Z",
            finished_at: "2026-10-14T01:01:00Z",
            output_persisted: false,
          },
        ],
        next_cursor: null,
      }),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(
      () => useRemoteMachineJobs("machine-1"),
      { wrapper: wrapper(client) },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.jobs[0]).toMatchObject({
      status: "finished",
      subject: { kind: "assistant", user_id: "owner-one", agent_id: null },
      started_at: null,
      exit_code: null,
      stop_reason: null,
      output_availability: "unavailable",
      output_expires_at: null,
      cancellation_available: false,
    });
    unmount();
    client.clear();
  });

  it("sends the current supported job date filters", async () => {
    transport.mockResolvedValueOnce(json({ jobs: [], next_cursor: null }));
    const client = makeClient();
    const { result, unmount } = renderHook(
      () => useRemoteMachineJobs("machine-1", { from: "2026-10-14T01:00:00Z" }),
      { wrapper: wrapper(client) },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(transport.mock.calls[0]?.[0]).toBe(
      "/api/v1/remote-machines/machine-1/jobs?limit=25&from=2026-10-14T01%3A00%3A00Z",
    );
    unmount();
    client.clear();
  });

  it("reads real owner confirmations without inventing reviewable parameters", async () => {
    transport.mockResolvedValueOnce(
      json({
        confirmations: [
          {
            id: "confirmation-one",
            node_id: "machine-1",
            principal: {
              kind: "oauth",
              user_id: "owner-one",
              oauth_client_id: "client-one",
            },
            grant_id: "grant-one",
            grant_revision: 1,
            assignment_revision: 2,
            runtime_generation: 3,
            operation: "shell.exec",
            parameters_digest: "digest",
            state: "approved",
            created_at: "2026-10-14T01:00:00Z",
            expires_at: "2026-10-14T01:10:00Z",
            decided_at: "2026-10-14T01:01:00Z",
            owner_only: true,
          },
        ],
      }),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(
      () => useRemoteMachineConfirmations("machine-1"),
      { wrapper: wrapper(client) },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(transport.mock.calls[0]?.[0]).toBe(
      "/api/v1/machine-confirmations?node_id=machine-1&limit=100",
    );
    expect(result.current.data?.confirmations[0]).toMatchObject({
      status: "approved",
      safe_parameters: [],
      review_parameters_available: false,
      approved_at: "2026-10-14T01:01:00Z",
      approved_until: "2026-10-14T01:10:00Z",
    });
    unmount();
    client.clear();
  });

  it("rejects confirmation metadata for another Machine", async () => {
    transport.mockResolvedValueOnce(
      json({
        confirmations: [
          {
            id: "confirmation-one",
            node_id: "machine-two",
            principal: {
              kind: "oauth",
              user_id: "owner-one",
              oauth_client_id: "client-one",
            },
            grant_id: "grant-one",
            grant_revision: 1,
            assignment_revision: 2,
            runtime_generation: 3,
            operation: "shell.exec",
            parameters_digest: "digest",
            state: "pending",
            created_at: "2026-10-14T01:00:00Z",
            expires_at: "2026-10-14T01:10:00Z",
            decided_at: null,
            owner_only: true,
          },
        ],
      }),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(
      () => useRemoteMachineConfirmations("machine-1"),
      { wrapper: wrapper(client) },
    );
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toBeUndefined();
    unmount();
    client.clear();
  });

  it("uses the mounted bound decision URI and exact reviewed body", async () => {
    transport.mockResolvedValueOnce(
      json({
        confirmation: {
          id: "confirmation-one",
          node_id: "machine-1",
          principal: {
            kind: "oauth",
            user_id: "owner-one",
            oauth_client_id: "client-one",
          },
          grant_id: "grant-one",
          grant_revision: 1,
          assignment_revision: 2,
          runtime_generation: 3,
          operation: "shell.exec",
          parameters_digest: "digest",
          status: "declined",
          subject: {
            kind: "oauth",
            user_id: "owner-one",
            oauth_client_id: "client-one",
          },
          safe_parameters: [],
          review_parameters_available: false,
          created_at: "2026-10-14T01:00:00Z",
          expires_at: "2026-10-14T01:10:00Z",
          approved_at: null,
          approved_until: null,
        },
      }),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(
      () => useDecideRemoteMachineConfirmation(),
      { wrapper: wrapper(client) },
    );
    await act(async () => {
      const decision = await result.current.mutateAsync({
        machineId: "machine-1",
        confirmationId: "confirmation-one",
        body: {
          approve: false,
          expected_grant_revision: 1,
          expected_runtime_generation: 3,
          parameters_digest: "digest",
        },
      });
      expect(decision).toMatchObject({
        id: "confirmation-one",
        status: "declined",
        review_parameters_available: false,
        safe_parameters: [],
      });
    });
    expect(transport.mock.calls[0]?.[0]).toBe(
      "/api/v1/remote-machines/machine-1/confirmations/confirmation-one/decision",
    );
    expect(transport.mock.calls[0]?.[1]?.method).toBe("POST");
    expect(JSON.parse(transport.mock.calls[0]?.[1]?.body as string)).toEqual({
      approve: false,
      expected_grant_revision: 1,
      expected_runtime_generation: 3,
      parameters_digest: "digest",
    });
    unmount();
    client.clear();
  });

  it("reads live output without putting raw text in Query or mutation caches", async () => {
    const output = {
      job_id: "job-1",
      status: "finished",
      exit_code: 0,
      stdout: "fixture-private-output",
      stderr: "",
      stdout_bytes: 22,
      stderr_bytes: 0,
      output_offset: 22,
      stderr_offset: 0,
      truncated: false,
      timed_out: false,
      duration_ms: 25,
    };
    transport.mockResolvedValueOnce(json(output));
    const client = makeClient();
    expect(
      await readRemoteMachineJobOutput("machine-1", "job-1", {
        output_offset: 0,
      }),
    ).toEqual(output);
    expect(client.getQueryCache().getAll()).toHaveLength(0);
    expect(client.getMutationCache().getAll()).toHaveLength(0);
    expect(transport.mock.calls[0]?.[1]?.headers).toMatchObject({
      "Cache-Control": "no-store",
    });
  });

  it("discards an output response after the authenticated account changes", async () => {
    let resolve!: (response: Response) => void;
    transport.mockImplementation(
      () =>
        new Promise<Response>((done) => {
          resolve = done;
        }),
    );
    const request = readRemoteMachineJobOutput("machine-1", "job-1");
    const rejected = expect(request).rejects.toThrow("account changed");
    await waitFor(() => expect(transport).toHaveBeenCalledTimes(1));
    useAuthStore.setState({ user: { id: "owner-two" } as User });
    resolve(json({ stdout: "old-owner-secret" }));
    await rejected;
  });
});

it("follows stable Machine pages including retained records and preserves unknown legacy completion", async () => {
  const cursor =
    "2026-10-14T01:00:00.000Z|f439233e-6dfe-44e4-8241-73d1da023111";
  transport.mockResolvedValueOnce(
    json({ machines: [remoteMachineFixture], next_cursor: cursor }),
  );
  transport.mockResolvedValueOnce(
    json({
      machines: [
        {
          ...remoteMachineFixture,
          node_id: "deleted-machine",
          resource_state: "deleted",
        },
      ],
      next_cursor: null,
    }),
  );
  const client = makeClient();
  const { result, unmount } = renderHook(() => useRemoteMachines(true, true), {
    wrapper: wrapper(client),
  });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data?.map((row) => row.node_id)).toEqual([
    "machine-1",
    "deleted-machine",
  ]);
  expect(result.current.discoveryComplete).toBe(true);
  expect(transport.mock.calls[0]?.[0]).toBe(
    "/api/v1/remote-machines?include_deleted=true",
  );
  expect(String(transport.mock.calls[1]?.[0])).toContain(
    new URLSearchParams({ cursor }).toString(),
  );
  unmount();
  client.clear();
});

it("rejects repeated Machine cursors instead of silently declaring complete discovery", async () => {
  transport.mockImplementation(async () =>
    json({ machines: [remoteMachineFixture], next_cursor: "repeat" }),
  );
  const client = makeClient();
  const { result, unmount } = renderHook(() => useRemoteMachines(), {
    wrapper: wrapper(client),
  });
  await waitFor(() => expect(result.current.isError).toBe(true));
  expect(result.current.data).toBeUndefined();
  expect(transport).toHaveBeenCalledTimes(2);
  unmount();
  client.clear();
});

it("preserves a composite lifecycle seek cursor without timestamp-only pagination", async () => {
  const cursor =
    "2026-10-14T01:00:00.000Z|f439233e-6dfe-44e4-8241-73d1da023111";
  transport.mockResolvedValueOnce(json({ events: [], next_cursor: cursor }));
  const client = makeClient();
  const { result, unmount } = renderHook(
    () => useRemoteMachineEvents("machine-1", cursor),
    { wrapper: wrapper(client) },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data?.next_cursor).toBe(cursor);
  expect(String(transport.mock.calls[0]?.[0])).toContain(
    new URLSearchParams({ cursor }).toString(),
  );
  unmount();
  client.clear();
});

it("passes global confirmation seek positions to the mounted owner endpoint", async () => {
  const cursor =
    "2026-10-14T01:00:00.000Z|f439233e-6dfe-44e4-8241-73d1da023111";
  transport.mockResolvedValueOnce(
    json({ confirmations: [], next_cursor: null }),
  );
  const client = makeClient();
  const { result, unmount } = renderHook(
    () => useRemoteMachineConfirmations("machine-1", cursor),
    { wrapper: wrapper(client) },
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(result.current.data?.next_cursor).toBeNull();
  expect(String(transport.mock.calls[0]?.[0])).toContain(
    new URLSearchParams({ cursor }).toString(),
  );
  unmount();
  client.clear();
});

it("sends the selected quote profile and keys entitlement preflight by that profile", async () => {
  transport.mockImplementation(async (url) =>
    String(url).includes("remote-machine-entitlement")
      ? json({
          eligible: true,
          free_slot_available: true,
          machine_id: null,
          reason: null,
          policy_version: "remote-machine-mvp",
          policy: remoteMachinePaidQuoteFixture.policy,
        })
      : json(remoteMachinePaidQuoteFixture),
  );
  const client = makeClient();
  const { result, unmount } = renderHook(
    () => ({
      quote: useRemoteMachineQuote(),
      entitlement: useRemoteMachineEntitlement(true, "remote-machine"),
    }),
    { wrapper: wrapper(client) },
  );
  await waitFor(() => expect(result.current.entitlement.isSuccess).toBe(true));
  await act(async () => {
    await result.current.quote.mutateAsync({
      free_slot: false,
      profile_version: "remote-machine",
    });
  });
  expect(
    transport.mock.calls.some(
      ([url]) =>
        url ===
        "/api/v1/remote-machine-entitlement?profile_version=remote-machine",
    ),
  ).toBe(true);
  const quoteCall = transport.mock.calls.find(
    ([url]) => url === "/api/v1/remote-machine-quotes",
  );
  expect(JSON.parse(quoteCall![1]!.body as string)).toEqual({
    free_slot: false,
    profile_version: "remote-machine",
  });
  expect(
    client.getQueryData([
      "remote-machine-entitlement",
      "owner-one",
      "remote-machine",
    ]),
  ).toBeDefined();
  unmount();
  client.clear();
});

describe("personal Remote Machine flag admission", () => {
  it.each(["absent", "off", "loading", "signed-out"])(
    "denies quote, create, resume and rekey before HTTP when %s",
    async (mode) => {
      useAuthStore.setState({
        user: {
          id: "owner-one",
          ...(mode !== "absent"
            ? {
                capabilities: {
                  enabled_features:
                    mode === "off" ? [] : ["experimental:remote-machines"],
                },
              }
            : {}),
        } as unknown as User,
        isLoading: mode === "loading",
        isAuthenticated: mode !== "signed-out",
      });
      const client = makeClient();
      const { result, unmount } = renderHook(
        () => ({
          quote: useRemoteMachineQuote(),
          create: useCreateRemoteMachine(),
          action: useRemoteMachineAction(),
        }),
        { wrapper: wrapper(client) },
      );
      await expect(
        result.current.quote.mutateAsync({ free_slot: true }),
      ).rejects.toThrow("Remote Machines are not enabled for this account.");
      await expect(
        result.current.create.mutateAsync({
          body: createBody,
          idempotencyKey: "flag-create",
        }),
      ).rejects.toThrow("Remote Machines are not enabled for this account.");
      for (const action of ["resume", "rekey"] as const) {
        await expect(
          result.current.action.mutateAsync({
            machineId: "machine-1",
            body: {
              action,
              expected_revision: 1,
              profile_version: "remote-machine",
              policy_version: "remote-machine-mvp",
              data_policy_version: "remote-machine-data",
              acknowledge_shared_environment: true,
            },
            idempotencyKey: `flag-${action}`,
          }),
        ).rejects.toThrow("Remote Machines are not enabled for this account.");
      }
      expect(transport).not.toHaveBeenCalled();
      unmount();
      client.clear();
    },
  );
});

it.each(["quote", "create", "resume", "rekey"])(
  "rechecks personal flag for queued %s dispatch",
  async (admission) => {
    let release!: () => void;
    const barrier = new Promise<void>((resolve) => {
      release = resolve;
    });
    const client = new QueryClient({
      mutationCache: new MutationCache({ onMutate: () => barrier }),
    });
    const { result, unmount } = renderHook(
      () => ({
        quote: useRemoteMachineQuote(),
        create: useCreateRemoteMachine(),
        action: useRemoteMachineAction(),
      }),
      { wrapper: wrapper(client) },
    );
    let request!: Promise<unknown>;
    act(() => {
      request =
        admission === "quote"
          ? result.current.quote.mutateAsync({ free_slot: false })
          : admission === "create"
            ? result.current.create.mutateAsync({
                body: createBody,
                idempotencyKey: "queued-create",
              })
            : result.current.action.mutateAsync({
                machineId: "machine-1",
                body: {
                  action: admission as "resume" | "rekey",
                  expected_revision: 1,
                  profile_version: "remote-machine",
                  policy_version: "remote-machine-mvp",
                  data_policy_version: "remote-machine-data",
                  acknowledge_shared_environment: true,
                },
                idempotencyKey: `queued-${admission}`,
              });
    });
    const rejected = expect(request).rejects.toThrow(
      "Remote Machines are not enabled for this account.",
    );
    act(() =>
      useAuthStore.setState({
        user: {
          id: "owner-one",
          capabilities: { enabled_features: [] },
        } as unknown as User,
      }),
    );
    await act(async () => {
      release();
      await rejected;
    });
    expect(transport).not.toHaveBeenCalled();
    unmount();
    client.clear();
  },
);

it.each(["pause", "delete"] as const)(
  "keeps %s HTTP dispatch available when the flag is off",
  async (action) => {
    useAuthStore.setState({
      user: {
        id: "owner-one",
        capabilities: { enabled_features: [] },
      } as unknown as User,
    });
    transport.mockResolvedValueOnce(
      json({
        operation: { ...remoteMachineOperationFixture, action },
        replayed: false,
      }),
    );
    const client = makeClient();
    const { result, unmount } = renderHook(useRemoteMachineAction, {
      wrapper: wrapper(client),
    });
    await act(async () => {
      await result.current.mutateAsync({
        machineId: "machine-1",
        body: {
          action,
          expected_revision: 1,
          ...(action === "delete"
            ? {
                data_policy_version: "remote-machine-data",
                acknowledge_data_loss: true,
              }
            : {}),
        },
        idempotencyKey: `flag-off-${action}`,
      });
    });
    expect(transport).toHaveBeenCalledTimes(1);
    expect(transport.mock.calls[0]?.[1]?.method).toBe(
      action === "delete" ? "DELETE" : "POST",
    );
    unmount();
    client.clear();
  },
);
