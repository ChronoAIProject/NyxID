import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AdminFeatureFlagsPage } from "./admin-feature-flags";

const { mockUseFlags, mockUseUsers } = vi.hoisted(() => ({
  mockUseFlags: vi.fn(),
  mockUseUsers: vi.fn(),
}));

vi.mock("@/hooks/use-admin-feature-flags", () => ({
  useAdminFeatureFlags: mockUseFlags,
  useSetAdminFeatureFlag: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useClearAdminFeatureFlag: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: mockUseUsers,
}));

beforeEach(() => {
  mockUseUsers.mockReturnValue({
    data: { users: [], total: 201, page: 1, per_page: 20 },
    isLoading: false,
  });
});

describe("AdminFeatureFlagsPage", () => {
  it("renders registry flags returned by the API", () => {
    mockUseFlags.mockReturnValue({
      data: {
        flags: [
          {
            key: "experimental:ai-assistant",
            description: "AI Assistant chat surface.",
            default_enabled: false,
            org_manageable: true,
            global_override: true,
            user_overrides: [],
          },
        ],
      },
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

  it("resolves an existing override label independently of search results", () => {
    mockUseFlags.mockReturnValue({
      data: {
        flags: [
          {
            key: "experimental:ai-assistant",
            description: "AI Assistant chat surface.",
            default_enabled: false,
            org_manageable: true,
            global_override: null,
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
          },
        ],
      },
      isLoading: false,
      error: null,
    });

    render(<AdminFeatureFlagsPage />);
    expect(screen.getByText("existing@example.com")).toBeInTheDocument();
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
      data: {
        flags: [
          {
            key: "experimental:ai-assistant",
            description: "AI Assistant chat surface.",
            default_enabled: false,
            org_manageable: true,
            global_override: null,
            user_overrides: [],
          },
        ],
      },
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
      expect(mockUseUsers).toHaveBeenCalledWith(1, 20, "late@example.com"),
    );
    expect(await screen.findByText("late@example.com")).toBeInTheDocument();
  });
});
