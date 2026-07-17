import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AdminFeatureFlagsPage } from "./admin-feature-flags";

const { mockUseFlags, mockUseUsers, mockSetFlag, mockClearFlag } = vi.hoisted(
  () => ({
    mockUseFlags: vi.fn(),
    mockUseUsers: vi.fn(),
    mockSetFlag: vi.fn(),
    mockClearFlag: vi.fn(),
  }),
);

vi.mock("@/hooks/use-admin-feature-flags", () => ({
  useAdminFeatureFlags: mockUseFlags,
  useSetAdminFeatureFlag: () => ({ mutateAsync: mockSetFlag, isPending: false }),
  useClearAdminFeatureFlag: () => ({
    mutateAsync: mockClearFlag,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: mockUseUsers,
}));

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

/**
 * Serves the unsearched person page (per_page 8) the picker asks for on focus,
 * and an empty page for every other call.
 */
function defaultSuggestionsMock() {
  return (
    _page: number,
    perPage: number,
    search?: string,
    kind?: string,
    options?: { enabled?: boolean },
  ) => {
    const isDefaults =
      perPage === 8 && !search && kind === "person" && options?.enabled;
    return {
      data: isDefaults
        ? {
            users: Array.from({ length: 8 }, (_, index) => ({
              id: `user-${index}`,
              email: `suggested-${index}@example.com`,
            })),
            total: 200,
            page: 1,
            per_page: 8,
          }
        : { users: [], total: 0, page: 1, per_page: perPage },
      isLoading: false,
    };
  };
}

beforeEach(() => {
  mockSetFlag.mockReset().mockResolvedValue(undefined);
  mockClearFlag.mockReset().mockResolvedValue(undefined);
  mockUseUsers.mockReset().mockReturnValue({
    data: { users: [], total: 201, page: 1, per_page: 20 },
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

  it("renders org overrides with their display name and slug", () => {
    mockUseFlags.mockReturnValue({
      data: {
        flags: [
          flagFixture({
            org_overrides: [
              {
                org_id: "org-1",
                org_display_name: "Acme Corp",
                org_slug: "acme",
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
    expect(screen.getByText("Acme Corp (acme)")).toBeInTheDocument();
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
      (_page: number, _perPage: number, search?: string, kind?: string) => ({
        data:
          kind === "person" && search === "late@example.com"
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

  it("suggests the first accounts with a remainder hint when search is focused", async () => {
    mockUseFlags.mockReturnValue({
      data: { flags: [flagFixture()] },
      isLoading: false,
      error: null,
    });
    mockUseUsers.mockImplementation(defaultSuggestionsMock());

    render(<AdminFeatureFlagsPage />);
    fireEvent.click(screen.getByText("experimental:ai-assistant"));
    // Nothing is suggested — and no request fans out — until the admin
    // actually opens the dropdown.
    expect(screen.queryByText("suggested-0@example.com")).not.toBeInTheDocument();
    expect(mockUseUsers).toHaveBeenCalledWith(1, 8, undefined, "person", {
      enabled: false,
    });

    fireEvent.focusIn(screen.getByLabelText("Search users by email"));

    // Over-fetches 8 and shows 5, so exclusions can't leave the list short.
    expect(
      await screen.findByText("suggested-0@example.com"),
    ).toBeInTheDocument();
    expect(screen.getByText("suggested-4@example.com")).toBeInTheDocument();
    expect(screen.queryByText("suggested-5@example.com")).not.toBeInTheDocument();
    // total (200) minus the 5 shown.
    expect(
      screen.getByText("+195 more — keep typing to narrow"),
    ).toBeInTheDocument();
    expect(mockUseUsers).toHaveBeenCalledWith(1, 8, undefined, "person", {
      enabled: true,
    });
  });

  it("stages an override from a default suggestion", async () => {
    mockUseFlags.mockReturnValue({
      data: { flags: [flagFixture()] },
      isLoading: false,
      error: null,
    });
    mockUseUsers.mockImplementation(defaultSuggestionsMock());

    render(<AdminFeatureFlagsPage />);
    fireEvent.click(screen.getByText("experimental:ai-assistant"));
    fireEvent.focusIn(screen.getByLabelText("Search users by email"));
    fireEvent.click(await screen.findByText("suggested-2@example.com"));

    fireEvent.click(screen.getByText("Apply changes"));
    await waitFor(() =>
      expect(mockSetFlag).toHaveBeenCalledWith({
        flagKey: "experimental:ai-assistant",
        body: { target_kind: "user", target_key: "user-2", enabled: true },
      }),
    );
    expect(mockSetFlag).toHaveBeenCalledTimes(1);
  });

  it("stages an org from the org search and applies an org-scoped override", async () => {
    mockUseFlags.mockReturnValue({
      data: { flags: [flagFixture()] },
      isLoading: false,
      error: null,
    });
    mockUseUsers.mockImplementation(
      (_page: number, _perPage: number, search?: string, kind?: string) => ({
        data:
          kind === "org" && search === "acme"
            ? {
                users: [
                  {
                    id: "org-9",
                    email: "org-acme@nyxid.local",
                    display_name: "Acme Corp",
                    slug: "acme",
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
      target: { value: "acme" },
    });
    fireEvent.click(await screen.findByText("Acme Corp"));

    // The pick stages an enabled org override; applying must send an
    // org-scoped write, never a global one (regression: NyxID killswitch).
    fireEvent.click(screen.getByText("Apply changes"));
    await waitFor(() =>
      expect(mockSetFlag).toHaveBeenCalledWith({
        flagKey: "experimental:ai-assistant",
        body: {
          target_kind: "org",
          target_key: "org-9",
          enabled: true,
        },
      }),
    );
    expect(mockSetFlag).toHaveBeenCalledTimes(1);
    expect(mockClearFlag).not.toHaveBeenCalled();
  });
});
