import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useCreateCrmView,
  useCrmView,
  useCrmViews,
  useDeleteCrmView,
  useUpdateCrmView,
  useValidateCrmView,
} from "@/hooks/use-crm-views";
import type { CrmViewEnvelope } from "@/lib/crm/views/schema";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

const { mockAssistantJson } = vi.hoisted(() => ({
  mockAssistantJson: vi.fn(),
}));

vi.mock("@/lib/assistant/assistant-http", () => ({
  assistantJson: mockAssistantJson,
}));

const envelope: CrmViewEnvelope = {
  formatVersion: 1,
  renderer: "json-render",
  rendererSpecVersion: "nyxid-crm-v1",
  queries: {},
  spec: {
    root: "main",
    elements: {
      main: {
        type: "Text",
        props: { content: "Hello", variant: "body" },
        children: [],
      },
    },
  },
};

const savedView = {
  id: "view-1",
  owner_id: "person-1",
  title: "Pipeline",
  revision: 3,
  envelope,
  valid: true,
  errors: [],
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T01:00:00Z",
};

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.setState({ user: { id: "person-1" } as User });
});

describe("CRM saved-view hooks", () => {
  it("lists and opens views through their distinct GET routes", async () => {
    mockAssistantJson
      .mockResolvedValueOnce({ views: [savedView] })
      .mockResolvedValueOnce(savedView);

    const listed = renderHook(() => useCrmViews(), { wrapper: wrapper() });
    await waitFor(() => expect(listed.result.current.isSuccess).toBe(true));
    expect(mockAssistantJson).toHaveBeenNthCalledWith(
      1,
      "/assistant/crm/views",
    );

    const opened = renderHook(() => useCrmView("view-1"), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(opened.result.current.isSuccess).toBe(true));
    expect(mockAssistantJson).toHaveBeenNthCalledWith(
      2,
      "/assistant/crm/views/view-1",
    );
  });

  it("validates the NyxID envelope before saving", async () => {
    mockAssistantJson.mockResolvedValue({ valid: true, errors: [] });
    const { result } = renderHook(() => useValidateCrmView(), {
      wrapper: wrapper(),
    });

    await result.current.mutateAsync(envelope);
    expect(mockAssistantJson).toHaveBeenCalledWith(
      "/assistant/crm/views/validate",
      { method: "POST", body: { envelope } },
    );
  });

  it("updates a view with its expected revision", async () => {
    mockAssistantJson.mockResolvedValue({ ...savedView, revision: 4 });
    const { result } = renderHook(() => useUpdateCrmView(), {
      wrapper: wrapper(),
    });

    await result.current.mutateAsync({
      id: "view-1",
      title: "Pipeline review",
      envelope,
      expectedRevision: 3,
      idempotencyKey: "update-view-action",
    });
    expect(mockAssistantJson).toHaveBeenCalledWith(
      "/assistant/crm/views/view-1",
      {
        method: "PUT",
        headers: { "Idempotency-Key": "update-view-action" },
        body: {
          title: "Pipeline review",
          envelope,
          expected_revision: 3,
        },
      },
    );
  });

  it("sends one Idempotency-Key for create and delete actions", async () => {
    mockAssistantJson
      .mockResolvedValueOnce(savedView)
      .mockResolvedValueOnce(undefined);
    const created = renderHook(() => useCreateCrmView(), {
      wrapper: wrapper(),
    });
    await created.result.current.mutateAsync({
      title: "Pipeline",
      envelope,
      idempotencyKey: "create-view-action",
    });
    expect(mockAssistantJson).toHaveBeenNthCalledWith(
      1,
      "/assistant/crm/views",
      {
        method: "POST",
        headers: { "Idempotency-Key": "create-view-action" },
        body: { title: "Pipeline", envelope },
      },
    );

    const deleted = renderHook(() => useDeleteCrmView(), {
      wrapper: wrapper(),
    });
    await deleted.result.current.mutateAsync({
      id: "view-1",
      idempotencyKey: "delete-view-action",
    });
    expect(mockAssistantJson).toHaveBeenNthCalledWith(
      2,
      "/assistant/crm/views/view-1",
      {
        method: "DELETE",
        headers: { "Idempotency-Key": "delete-view-action" },
        body: {},
      },
    );
  });
});
