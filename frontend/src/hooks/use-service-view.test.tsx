import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { User } from "@/types/api";
import {
  DEFAULT_SERVICE_FILTERS,
  type ServiceViewFilters,
} from "@/schemas/service-view";

const { put } = vi.hoisted(() => ({ put: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { put } }));
vi.mock("@/lib/telemetry", () => ({ identify: vi.fn(), reset: vi.fn() }));

import { useAuthStore } from "@/stores/auth-store";
import { useServiceCardView } from "@/stores/service-card-view-store";
import { useServiceView } from "./use-service-view";

function user(id = "user-a", saved: ServiceViewFilters | null = null): User {
  return {
    id,
    email: `${id}@example.com`,
    display_name: null,
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-01-01",
    profile_config: {
      onboarding: { ai_services_completed_at: "2026-01-01" },
      services_view: saved,
    },
  };
}
function mount() {
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return {
    client,
    ...renderHook(useServiceView, {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    }),
  };
}
const saved: ServiceViewFilters = {
  ...DEFAULT_SERVICE_FILTERS,
  source: "org",
  search: "team",
};

beforeEach(() => {
  vi.resetAllMocks();
  useAuthStore.setState({ user: user(), isAuthenticated: true });
  useServiceCardView.setState({
    accountId: undefined,
    expanded: [],
    filters: undefined,
  });
});
afterEach(cleanup);

describe("account service view preferences", () => {
  it("starts personal, honors saved All services, and restores only the newest expanded card", () => {
    const { result } = mount();
    expect(result.current.filters.source).toBe("personal");
    act(() =>
      useAuthStore.setState({
        user: user("user-a", { ...DEFAULT_SERVICE_FILTERS, source: "all" }),
      }),
    );
    expect(result.current.filters.source).toBe("all");
    act(() => result.current.setExpanded(["first", "latest"]));
    expect(result.current.expanded).toEqual(["latest"]);
  });

  it("loads defaults, saves only on request, and restores them on a fresh visit", async () => {
    useAuthStore.setState({ user: user("user-a", saved) });
    put.mockImplementation(async (_path, body) => body);
    const mounted = mount();
    expect(mounted.result.current.filters).toEqual(saved);
    expect(mounted.result.current.expanded).toEqual([]);
    const next = {
      ...saved,
      state: "disabled" as const,
      organization_ids: ["org-1", "org-2"],
      service_group_ids: ["catalog:openai", "catalog:codex"],
    };
    act(() => mounted.result.current.setFilters(next));
    expect(put).not.toHaveBeenCalled();
    act(() => mounted.result.current.saveDefault());
    await waitFor(() => expect(mounted.result.current.isDefault).toBe(true));
    expect(put).toHaveBeenCalledWith("/users/me/preferences/services", next);
    expect(
      mounted.client.getQueryData<User>(["user", "me"])?.profile_config
        ?.services_view,
    ).toEqual(next);
    expect(
      useAuthStore.getState().user?.profile_config?.onboarding
        .ai_services_completed_at,
    ).toBe("2026-01-01");
    mounted.unmount();
    useServiceCardView.setState({
      accountId: undefined,
      filters: undefined,
      expanded: [],
    });
    const fresh = mount();
    expect(fresh.result.current.filters).toEqual(next);
    act(() => fresh.result.current.setFilters(DEFAULT_SERVICE_FILTERS));
    expect(fresh.result.current.differsFromDefault).toBe(true);
    act(() => fresh.result.current.restoreDefault());
    expect(fresh.result.current.filters).toEqual(next);
  });

  it("keeps the previous default when saving fails and allows a retry", async () => {
    useAuthStore.setState({ user: user("user-a", saved) });
    put
      .mockRejectedValueOnce(new Error("offline"))
      .mockImplementation(async (_path, body) => body);
    const { result } = mount();
    act(() => result.current.setFilters(DEFAULT_SERVICE_FILTERS));
    act(() => result.current.saveDefault());
    await waitFor(() =>
      expect(result.current.saveError).toMatch(/Could not save/),
    );
    expect(result.current.isDefault).toBe(false);
    expect(useAuthStore.getState().user?.profile_config?.services_view).toEqual(
      saved,
    );
    act(() => result.current.saveDefault());
    await waitFor(() => expect(result.current.isDefault).toBe(true));
    expect(result.current.saveError).toBeNull();
  });

  it("does not overwrite draft filters when profile data arrives late", () => {
    const oldUser = user();
    useAuthStore.setState({ user: { ...oldUser, profile_config: undefined } });
    const { result } = mount();
    expect(result.current.canSave).toBe(false);
    act(() =>
      result.current.setFilters({
        ...DEFAULT_SERVICE_FILTERS,
        search: "my draft",
      }),
    );
    act(() => useAuthStore.setState({ user: user("user-a", saved) }));
    expect(result.current.filters.search).toBe("my draft");
    expect(result.current.canSave).toBe(true);
  });

  it("ignores an old account's save response after switching accounts", async () => {
    let resolveSave!: (value: ServiceViewFilters) => void;
    put.mockImplementation(
      () =>
        new Promise<ServiceViewFilters>((resolve) => {
          resolveSave = resolve;
        }),
    );
    const { result, client } = mount();
    act(() => result.current.setFilters(saved));
    act(() => result.current.setExpanded(["catalog:openai"]));
    act(() => result.current.saveDefault());
    await waitFor(() => expect(put).toHaveBeenCalledOnce());
    act(() => useAuthStore.getState().setUser(user("user-b")));
    expect(result.current.filters).toEqual(DEFAULT_SERVICE_FILTERS);
    expect(result.current.expanded).toEqual([]);
    await act(async () => resolveSave(saved));
    expect(useAuthStore.getState().user?.id).toBe("user-b");
    expect(
      useAuthStore.getState().user?.profile_config?.services_view,
    ).toBeNull();
    expect(client.getQueryData(["user", "me"])).toBeUndefined();
    expect(result.current.saveError).toBeNull();
  });

  it("keeps edits made while saving and clears drafts on sign-out", async () => {
    let resolveSave!: (value: ServiceViewFilters) => void;
    put.mockImplementation(
      () =>
        new Promise<ServiceViewFilters>((resolve) => {
          resolveSave = resolve;
        }),
    );
    const { result } = mount();
    act(() => result.current.setFilters(saved));
    act(() => result.current.saveDefault());
    await waitFor(() => expect(put).toHaveBeenCalledOnce());
    act(() => result.current.setFilters({ ...saved, search: "new search" }));
    await act(async () => resolveSave(saved));
    expect(result.current.filters.search).toBe("new search");
    expect(result.current.isDefault).toBe(false);
    act(() => useAuthStore.getState().setUser(null));
    act(() => useAuthStore.getState().setUser(user("user-a", saved)));
    expect(result.current.filters).toEqual(saved);
  });

  it("does not attempt account saves against older servers", () => {
    useAuthStore.setState({ user: { ...user(), profile_config: undefined } });
    const { result } = mount();
    expect(result.current.canSave).toBe(false);
    act(() => result.current.saveDefault());
    expect(put).not.toHaveBeenCalled();
  });
});
