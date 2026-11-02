import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { machineContextReviewFixture as review } from "@/test/machine-access-fixture";
import {
  usePrepareMachineContext,
  useMachineContextReview,
  useCommitMachineContext,
} from "./use-machine-access";

const { read } = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: read }));
let client: QueryClient;
beforeEach(() => {
  read.mockReset().mockResolvedValue(review);
  useAuthStore.setState({ user: { id: "owner" } as User });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});
afterEach(() => client.clear());
const wrapper = ({ children }: PropsWithChildren) => (
  <QueryClientProvider client={client}>{children}</QueryClientProvider>
);

it("prepares only explicit mode/version/consent, reads the stored review, then commits without any Selection", async () => {
  const { result } = renderHook(
    () => ({
      prepare: usePrepareMachineContext("agent"),
      commit: useCommitMachineContext("agent"),
    }),
    { wrapper },
  );
  await act(async () => {
    await result.current.prepare.mutateAsync({
      node: "node",
      body: {
        mode: "separated",
        policy_version: "machine-context-mode-v1",
        acknowledge_separation: true,
      },
    });
  });
  expect(read).toHaveBeenLastCalledWith(
    "/assistant/nyxagent/agents/agent/machines/node/context-prepare",
    expect.objectContaining({
      method: "POST",
      body: {
        mode: "separated",
        policy_version: "machine-context-mode-v1",
        acknowledge_separation: true,
      },
    }),
  );
  const get = renderHook(
    () => useMachineContextReview("agent", "node", "review-1"),
    { wrapper },
  );
  await waitFor(() =>
    expect(get.result.current.data?.capabilities).toEqual(review.capabilities),
  );
  expect(read).toHaveBeenLastCalledWith(
    "/assistant/nyxagent/agents/agent/machines/node/context-reviews/review-1",
  );
  read.mockResolvedValue({
    ...review,
    status: "committed",
    committed_revision: 8,
  });
  await act(async () => {
    await result.current.commit.mutateAsync({
      node: "node",
      reviewId: "review-1",
    });
  });
  expect(
    read.mock.calls.some(
      ([path, request]) =>
        path.endsWith("/review-1/commit") &&
        JSON.stringify(request.body) === "{}",
    ),
  ).toBe(true);
  await waitFor(() =>
    expect(get.result.current.data?.status).toBe("committed"),
  );
});

it("rejects account switches before preparing or committing and does not send a stale owner request", async () => {
  const { result } = renderHook(
    () => ({
      prepare: usePrepareMachineContext("agent"),
      commit: useCommitMachineContext("agent"),
    }),
    { wrapper },
  );
  const prepared = result.current.prepare.mutateAsync;
  const commit = result.current.commit.mutateAsync;
  act(() => useAuthStore.setState({ user: { id: "other-owner" } as User }));
  await expect(
    prepared({
      node: "node",
      body: {
        mode: "shared_legacy",
        policy_version: "machine-context-mode-v1",
        acknowledge_separation: true,
      },
    }),
  ).rejects.toThrow(/account changed/);
  await expect(commit({ node: "node", reviewId: "review-1" })).rejects.toThrow(
    /account changed/,
  );
  expect(read).not.toHaveBeenCalled();
});

it("rejects a stored snapshot belonging to a different Agent or Machine", async () => {
  read.mockResolvedValue({ ...review, node_id: "different-node" });
  const { result } = renderHook(() => usePrepareMachineContext("agent"), {
    wrapper,
  });
  await act(async () => {
    await expect(
      result.current.mutateAsync({
        node: "node",
        body: {
          mode: "separated",
          policy_version: "machine-context-mode-v1",
          acknowledge_separation: true,
        },
      }),
    ).rejects.toThrow(/review target/);
  });
});
