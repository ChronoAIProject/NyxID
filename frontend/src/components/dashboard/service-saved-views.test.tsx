import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DEFAULT_SERVICE_FILTERS } from "@/schemas/service-view";
import type { User } from "@/types/api";

const { put } = vi.hoisted(() => ({ put: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { put } }));
vi.mock("@/lib/telemetry", () => ({ identify: vi.fn(), reset: vi.fn() }));
import { useAuthStore } from "@/stores/auth-store";
import { useServiceCardView } from "@/stores/service-card-view-store";
import { useServiceView } from "@/hooks/use-service-view";
import { ServiceSavedViews } from "./service-saved-views";

function Harness() {
  const view = useServiceView();
  return (
    <ServiceSavedViews
      view={view}
      onRestore={(filters, viewId) => {
        if (viewId) view.restoreSavedView(viewId);
        else if (filters) view.setFilters(filters);
      }}
    />
  );
}

beforeEach(() => {
  put.mockReset().mockImplementation(async (_path, body) => body);
  useAuthStore.setState({
    user: {
      id: "user-a",
      email: "test@example.com",
      display_name: null,
      avatar_url: null,
      email_verified: true,
      mfa_enabled: false,
      is_admin: false,
      is_active: true,
      created_at: "2026-01-01",
      profile_config: {
        services_view: null,
        service_views: null,
        onboarding: { ai_services_completed_at: null },
      },
    } satisfies User,
  });
  useServiceCardView.setState({
    accountId: undefined,
    filters: undefined,
    savedViewId: undefined,
    expanded: [],
  });
});
afterEach(cleanup);

it("saves multiple named views, selects a default, and confirms deletion", async () => {
  const user = userEvent.setup();
  render(
    <QueryClientProvider client={new QueryClient()}>
      <Harness />
    </QueryClientProvider>,
  );
  await user.click(screen.getByRole("button", { name: "Saved views" }));
  await user.type(screen.getByLabelText("Save as a new view"), "Personal");
  await user.click(screen.getByRole("button", { name: "Save new view" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Set as default: Personal" }),
    ).toBeEnabled(),
  );
  await user.clear(screen.getByLabelText("Save as a new view"));
  await user.type(screen.getByLabelText("Save as a new view"), "Another");
  await user.click(screen.getByRole("button", { name: "Save new view" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Set as default: Another" }),
    ).toBeEnabled(),
  );
  await user.click(
    screen.getByRole("button", { name: "Set as default: Another" }),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Remove default: Another" }),
    ).toBeEnabled(),
  );
  await user.click(
    screen.getByRole("button", { name: "Delete view: Another" }),
  );
  expect(put).toHaveBeenCalledTimes(3);
  await user.click(screen.getByRole("button", { name: "Delete" }));
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Delete view: Another" }),
    ).not.toBeInTheDocument(),
  );
  expect(
    useAuthStore.getState().user?.profile_config?.services_view,
  ).toBeNull();
  expect(
    useAuthStore.getState().user?.profile_config?.service_views?.views[0]
      ?.filters,
  ).toEqual(DEFAULT_SERVICE_FILTERS);
});

it("restores and updates the selected view without overwriting another view", async () => {
  const current = useAuthStore.getState().user!;
  useAuthStore.setState({
    user: {
      ...current,
      profile_config: {
        ...current.profile_config!,
        service_views: {
          views: [
            {
              id: "personal",
              name: "Personal",
              filters: DEFAULT_SERVICE_FILTERS,
            },
            {
              id: "team",
              name: "Team",
              filters: { ...DEFAULT_SERVICE_FILTERS, search: "team" },
            },
          ],
          default_id: "personal",
        },
      },
    },
  });
  const user = userEvent.setup();
  const mounted = render(
    <QueryClientProvider client={new QueryClient()}>
      <Harness />
    </QueryClientProvider>,
  );
  await user.click(screen.getByRole("button", { name: "Saved views" }));
  await user.click(screen.getByRole("button", { name: /^Team/ }));
  expect(useServiceCardView.getState().filters?.search).toBe("team");
  expect(screen.getByRole("status")).toHaveTextContent("Team");
  act(() => useServiceCardView.setState({ filters: DEFAULT_SERVICE_FILTERS }));
  mounted.unmount();
  render(
    <QueryClientProvider client={new QueryClient()}>
      <Harness />
    </QueryClientProvider>,
  );
  await user.click(screen.getByRole("button", { name: "Saved views" }));
  await user.click(screen.getByRole("button", { name: /Update “Team”/ }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: /Update “Team”/ }),
    ).toBeDisabled(),
  );
  expect(
    useAuthStore.getState().user?.profile_config?.service_views?.views[1]
      ?.filters,
  ).toEqual(DEFAULT_SERVICE_FILTERS);
  await user.click(screen.getByRole("button", { name: /^Team/ }));
  expect(screen.getByRole("status")).toHaveTextContent("Team");
});
