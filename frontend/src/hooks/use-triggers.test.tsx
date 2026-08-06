import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useCreateTrigger,
  useDeleteTrigger,
  useRotateTriggerSecret,
  useTrigger,
  useTriggers,
  useUpdateTrigger,
} from "./use-triggers";

const { mockDelete, mockGet, mockPatch, mockPost } = vi.hoisted(() => ({
  mockDelete: vi.fn(),
  mockGet: vi.fn(),
  mockPatch: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { delete: mockDelete, get: mockGet, patch: mockPatch, post: mockPost },
}));

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

const trigger = {
  id: "80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  user_id: "1ca34962-7698-46a0-85d0-c85445becd72",
  label: "Repository activity",
  user_service_id: null,
  status: "active",
  verification: { mode: "token", location: "bearer" },
  delivery: { type: "notification" },
  inbound_url:
    "https://api.example.com/api/v1/webhooks/triggers/80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  created_at: "2026-08-06T09:30:00.123+00:00",
  updated_at: "2026-08-06T09:30:00.123+00:00",
};

beforeEach(() => vi.clearAllMocks());

describe("trigger query hooks", () => {
  it("lists personal and organization triggers using backend query names", async () => {
    mockGet.mockResolvedValue({ triggers: [trigger] });
    const personal = renderHook(() => useTriggers(), { wrapper: wrapper() });
    await waitFor(() => expect(personal.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/triggers");

    const organization = renderHook(() => useTriggers("org/1"), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(organization.result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/triggers?org_id=org%2F1");
  });

  it("gets one trigger with an encoded id", async () => {
    mockGet.mockResolvedValue(trigger);
    const { result } = renderHook(() => useTrigger("trigger/1"), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/triggers/trigger%2F1");
  });
});

describe("trigger mutation hooks", () => {
  it("creates, updates, rotates, and deletes with exact wire bodies and routes", async () => {
    mockPost
      .mockResolvedValueOnce({
        trigger,
        secret: "nyx_trg_once",
        delivery_signing_secret: null,
      })
      .mockResolvedValueOnce({ trigger, secret: "nyx_trg_rotated" });
    mockPatch.mockResolvedValue({ trigger, delivery_signing_secret: null });
    mockDelete.mockResolvedValue({ message: "Trigger deleted" });

    const create = renderHook(() => useCreateTrigger(), { wrapper: wrapper() });
    await create.result.current.mutateAsync({
      label: "Repository activity",
      verification: { mode: "token", location: "bearer" },
      delivery: { type: "notification" },
    });
    expect(mockPost).toHaveBeenNthCalledWith(1, "/triggers", {
      label: "Repository activity",
      verification: { mode: "token", location: "bearer" },
      delivery: { type: "notification" },
    });

    const update = renderHook(() => useUpdateTrigger(), { wrapper: wrapper() });
    await update.result.current.mutateAsync({
      id: "trigger/1",
      data: { status: "disabled" },
    });
    expect(mockPatch).toHaveBeenCalledWith("/triggers/trigger%2F1", {
      status: "disabled",
    });

    const rotate = renderHook(() => useRotateTriggerSecret(), {
      wrapper: wrapper(),
    });
    await rotate.result.current.mutateAsync("trigger/1");
    expect(mockPost).toHaveBeenNthCalledWith(
      2,
      "/triggers/trigger%2F1/rotate-secret",
    );

    const remove = renderHook(() => useDeleteTrigger(), { wrapper: wrapper() });
    await remove.result.current.mutateAsync("trigger/1");
    expect(mockDelete).toHaveBeenCalledWith("/triggers/trigger%2F1");
  });
});
