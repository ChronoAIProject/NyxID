import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AdminFeatureFlagsPage } from "./admin-feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

const { mockUseFlags, mockUseUsers, mockSetOverride, mockClearOverride } =
  vi.hoisted(() => ({
    mockUseFlags: vi.fn(),
    mockUseUsers: vi.fn(),
    mockSetOverride: vi.fn(),
    mockClearOverride: vi.fn(),
  }));

vi.mock("@/hooks/use-admin-feature-flags", () => ({
  useAdminFeatureFlags: mockUseFlags,
  useSetAdminFeatureFlag: () => ({
    mutateAsync: mockSetOverride,
    isPending: false,
  }),
  useClearAdminFeatureFlag: () => ({
    mutateAsync: mockClearOverride,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: mockUseUsers,
}));

function adminUser(): User {
  return {
    id: "admin-1",
    email: "admin@example.com",
    display_name: "Admin",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: true,
    role: "admin",
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
  };
}

function operatorUser(): User {
  return {
    ...adminUser(),
    id: "operator-1",
    email: "operator@example.com",
    display_name: "Operator",
    is_admin: false,
    is_operator: true,
    role: "operator",
  };
}

function flagFixture(
  overrides: Partial<{
    global_override: boolean | null;
    org_overrides: unknown[];
    user_overrides: unknown[];
  }> = {},
) {
  return {
    key: "experimental:ai-assistant",
    description: "AI Assistant chat surface.",
    default_enabled: false,
    org_manageable: true,
    global_override: null,
    org_overrides: [],
    user_overrides: [],
    ...overrides,
  };
}

beforeEach(() => {
  mockSetOverride.mockReset().mockResolvedValue(undefined);
  mockClearOverride.mockReset().mockResolvedValue(undefined);
  mockUseUsers.mockReset().mockReturnValue({
    data: { users: [], total: 201, page: 1, per_page: 20 },
    isLoading: false,
  });
  useAuthStore.setState({
    user: adminUser(),
    isAuthenticated: true,
    isLoading: false,
  });
});

describe("AdminFeatureFlagsPage", () => {
  it("renders registry flags returned by the API", () => {
    mockUseFlags.mockReturnValue({
      data: { flags: [flagFixture({ global_override: true })] },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    expect(screen.getByText("experimental:ai-assistant")).toBeInTheDocument();
    expect(screen.getByText("Default: Disabled")).toBeInTheDocument();
    expect(screen.queryByText("No feature flags")).not.toBeInTheDocument();
  });

  it("shows the empty state only for an empty registry response", () => {
    mockUseFlags.mockReturnValue({
      data: { flags: [] },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    expect(screen.getByText("No feature flags")).toBeInTheDocument();
  });

  it("tolerates a backend response without org_overrides (deploy skew)", () => {
    const legacy = flagFixture() as Record<string, unknown>;
    delete legacy["org_overrides"];
    mockUseFlags.mockReturnValue({
      data: { flags: [legacy] },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    expect(screen.getByText("experimental:ai-assistant")).toBeInTheDocument();
  });

  it("resolves an existing override label independently of search results", () => {
    mockUseFlags.mockReturnValue({
      data: {
        flags: [
          flagFixture({
            user_overrides: [
              {
                user_id: "user-outside-page",
                user_email: "existing@example.com",
                user_display_name: null,
                enabled: true,
                updated_at: "2026-07-17T00:00:00Z",
                updated_by: "admin-1",
              },
            ],
          }),
        ],
      },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    expect(screen.getByText("existing@example.com")).toBeInTheDocument();
  });

  it("renders org overrides with their display labels", () => {
    mockUseFlags.mockReturnValue({
      data: {
        flags: [
          flagFixture({
            org_overrides: [
              {
                org_user_id: "org-1",
                org_display_name: "ChronoAI",
                org_slug: "chronoai",
                enabled: true,
                updated_at: "2026-07-17T00:00:00Z",
                updated_by: "admin-1",
              },
            ],
          }),
        ],
      },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    expect(screen.getByText("ChronoAI (chronoai)")).toBeInTheDocument();
  });

  it("shows loading and error states", () => {
    mockUseFlags.mockReturnValue({ data: undefined, isLoading: true, error: null });
    const { rerender } = render(<AdminFeatureFlagsPage />);
    expect(screen.getByLabelText("Loading feature flags")).toBeInTheDocument();

    mockUseFlags.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("failed"),
    });
    rerender(<AdminFeatureFlagsPage />);
    expect(screen.getByText("Failed to load feature flags")).toBeInTheDocument();
  });

  it("finds a user beyond the unsearched first page with server search", async () => {
    mockUseFlags.mockReturnValue({
      data: { flags: [flagFixture()] },
      isLoading: false,
      error: null,
    });
    mockUseUsers.mockImplementation(
      (_page: number, _perPage: number, search?: string) => ({
        data:
          search === "late@example.com"
            ? {
                users: [{ id: "user-201", email: "late@example.com" }],
                total: 1,
                page: 1,
                per_page: 20,
              }
            : { users: [], total: 201, page: 1, per_page: 20 },
        isLoading: false,
      }),
    );

    render(<AdminFeatureFlagsPage />);
    fireEvent.click(screen.getByText("experimental:ai-assistant"));
    fireEvent.change(screen.getByLabelText("Search users by email"), {
      target: { value: "late@example.com" },
    });

    await waitFor(() =>
      expect(mockUseUsers).toHaveBeenCalledWith(
        1,
        20,
        "late@example.com",
        "person",
      ),
    );
    expect(await screen.findByText("late@example.com")).toBeInTheDocument();
  });

  it("stages an org from the org search and submits an org-scoped write", async () => {
    mockUseFlags.mockReturnValue({
      data: { flags: [flagFixture()] },
      isLoading: false,
      error: null,
    });
    mockUseUsers.mockImplementation(
      (
        _page: number,
        _perPage: number,
        search?: string,
        userType?: string,
      ) => ({
        data:
          userType === "org" && search === "chrono"
            ? {
                users: [
                  {
                    id: "org-1",
                    email: "org@example.com",
                    display_name: "ChronoAI",
                    slug: "chronoai",
                    user_type: "org",
                  },
                ],
                total: 1,
                page: 1,
                per_page: 20,
              }
            : { users: [], total: 0, page: 1, per_page: 20 },
        isLoading: false,
      }),
    );

    render(<AdminFeatureFlagsPage />);
    fireEvent.click(screen.getByText("experimental:ai-assistant"));
    fireEvent.change(screen.getByLabelText("Search organizations by name"), {
      target: { value: "chrono" },
    });
    fireEvent.click(await screen.findByText("ChronoAI"));

    fireEvent.click(screen.getByText("Apply changes"));

    // The regression this page shipped with: an org draft must never be
    // widened to a global write.
    await waitFor(() => expect(mockSetOverride).toHaveBeenCalledTimes(1));
    expect(mockSetOverride).toHaveBeenCalledWith({
      flagKey: "experimental:ai-assistant",
      body: {
        target_kind: "org",
        target_key: "org-1",
        enabled: true,
      },
    });
    expect(mockClearOverride).not.toHaveBeenCalled();
  });

  it("clears an org override with an org-scoped delete", async () => {
    mockUseFlags.mockReturnValue({
      data: {
        flags: [
          flagFixture({
            org_overrides: [
              {
                org_user_id: "org-1",
                org_display_name: "ChronoAI",
                org_slug: "chronoai",
                enabled: true,
                updated_at: "2026-07-17T00:00:00Z",
                updated_by: "admin-1",
              },
            ],
          }),
        ],
      },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    fireEvent.click(screen.getByText("experimental:ai-assistant"));
    fireEvent.click(screen.getByLabelText("ChronoAI (chronoai)"));
    fireEvent.click(await screen.findByRole("option", { name: "Inherit" }));
    fireEvent.click(screen.getByText("Apply changes"));

    await waitFor(() => expect(mockClearOverride).toHaveBeenCalledTimes(1));
    expect(mockClearOverride).toHaveBeenCalledWith({
      flagKey: "experimental:ai-assistant",
      targetKind: "org",
      targetKey: "org-1",
    });
    expect(mockSetOverride).not.toHaveBeenCalled();
  });

  it("shows operators a read-only view without pickers or apply controls", () => {
    useAuthStore.setState({ user: operatorUser() });
    mockUseFlags.mockReturnValue({
      data: {
        flags: [
          flagFixture({
            org_overrides: [
              {
                org_user_id: "org-1",
                org_display_name: "ChronoAI",
                org_slug: "chronoai",
                enabled: true,
                updated_at: "2026-07-17T00:00:00Z",
                updated_by: "admin-1",
              },
            ],
          }),
        ],
      },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    fireEvent.click(screen.getByText("experimental:ai-assistant"));

    expect(
      screen.queryByLabelText("Search organizations by name"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("Search users by email"),
    ).not.toBeInTheDocument();
    // Scope selects are disabled, so no draft can be staged.
    expect(screen.getByLabelText("ChronoAI (chronoai)")).toBeDisabled();
    expect(
      screen.getByLabelText("All users (rollout / killswitch)"),
    ).toBeDisabled();
    expect(screen.queryByText("Apply changes")).not.toBeInTheDocument();
  });
});
