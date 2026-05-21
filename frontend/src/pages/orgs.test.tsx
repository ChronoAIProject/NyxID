import type { ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OrgListItem } from "@/schemas/orgs";

const { fixtures, mockNavigate, mockCreateOrg, mockRefetch } = vi.hoisted(
  () => ({
    fixtures: {
      orgs: [] as OrgListItem[] | undefined,
      isLoading: false,
      error: null as unknown,
    },
    mockNavigate: vi.fn(),
    mockCreateOrg: vi.fn(),
    mockRefetch: vi.fn(),
  }),
);

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    params,
    ...props
  }: {
    readonly children: ReactNode;
    readonly to: string;
    readonly params?: Record<string, string>;
  }) => (
    <a href={to} data-org-id={params?.orgId} {...props}>
      {children}
    </a>
  ),
  useNavigate: () => mockNavigate,
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({
    data: fixtures.orgs,
    isLoading: fixtures.isLoading,
    error: fixtures.error,
    refetch: mockRefetch,
  }),
  // Consumed by the real CreateOrgDialog rendered inside OrgsPage.
  useCreateOrg: () => ({
    mutateAsync: mockCreateOrg,
    isPending: false,
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock("@/components/orgs/org-avatar", () => ({
  OrgAvatar: () => <div aria-hidden="true" />,
}));

import { OrgsPage } from "./orgs";

function makeOrg(overrides: Partial<OrgListItem> = {}): OrgListItem {
  return {
    id: "org-1",
    slug: "acme",
    display_name: "Acme Inc.",
    avatar_url: null,
    contact_email: "team@acme.test",
    your_role: "admin",
    created_at: "2026-04-20T00:00:00Z",
    ...overrides,
  };
}

describe("OrgsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fixtures.orgs = [];
    fixtures.isLoading = false;
    fixtures.error = null;
  });

  it("renders the org list with name, slug and role for each org", () => {
    fixtures.orgs = [
      makeOrg({ id: "org-1", display_name: "Acme Inc.", slug: "acme" }),
      makeOrg({
        id: "org-2",
        display_name: "Globex",
        slug: "globex",
        your_role: "member",
        contact_email: null,
      }),
    ];

    render(<OrgsPage />);

    expect(screen.getByText("Acme Inc.")).toBeInTheDocument();
    expect(screen.getByText("@acme")).toBeInTheDocument();
    expect(screen.getByText("Globex")).toBeInTheDocument();
    expect(screen.getByText("@globex")).toBeInTheDocument();
    // Role badges reflect each org's your_role.
    expect(screen.getByText("Admin")).toBeInTheDocument();
    expect(screen.getByText("Member")).toBeInTheDocument();
    // Org cards link to the detail route with the org id param.
    const acmeLink = screen.getByText("Acme Inc.").closest("a");
    expect(acmeLink).toHaveAttribute("data-org-id", "org-1");
  });

  it("shows the empty state when there are no orgs and opens create from its CTA", async () => {
    const user = userEvent.setup();
    fixtures.orgs = [];

    render(<OrgsPage />);

    expect(screen.getByText("No organizations yet")).toBeInTheDocument();

    // The empty-state CTA is the only path to create when the list is empty.
    await user.click(
      screen.getByRole("button", { name: "Create your first organization" }),
    );

    expect(
      screen.getByRole("heading", { name: "Create Organization" }),
    ).toBeInTheDocument();
  });

  it("creates an org from the header CTA: opens dialog, submits, calls useCreateOrg with the form body", async () => {
    const user = userEvent.setup();
    fixtures.orgs = [makeOrg()];
    mockCreateOrg.mockResolvedValue({ id: "org-99", display_name: "New Co" });

    render(<OrgsPage />);

    // Dialog is closed initially.
    expect(
      screen.queryByRole("heading", { name: "Create Organization" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New Organization" }));

    expect(
      screen.getByRole("heading", { name: "Create Organization" }),
    ).toBeInTheDocument();

    await user.type(screen.getByLabelText("Display name"), "New Co");
    await user.type(
      screen.getByLabelText("Contact email (optional)"),
      "hello@newco.test",
    );
    await user.click(
      screen.getByRole("button", { name: "Create Organization" }),
    );

    await waitFor(() => {
      expect(mockCreateOrg).toHaveBeenCalledWith({
        display_name: "New Co",
        contact_email: "hello@newco.test",
        avatar_url: "",
      });
    });
    // Success path navigates to the freshly-created org's detail page.
    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/orgs/$orgId",
        params: { orgId: "org-99" },
      });
    });
  });

  it("disables the create submit until a display name is entered", async () => {
    const user = userEvent.setup();
    fixtures.orgs = [makeOrg()];

    render(<OrgsPage />);

    await user.click(screen.getByRole("button", { name: "New Organization" }));

    const submit = screen.getByRole("button", { name: "Create Organization" });
    expect(submit).toBeDisabled();

    await user.type(screen.getByLabelText("Display name"), "X");
    expect(submit).toBeEnabled();
  });

  it("shows the error banner with a retry when the list query fails", async () => {
    const user = userEvent.setup();
    fixtures.error = new Error("boom");

    render(<OrgsPage />);

    expect(
      screen.getByText("Failed to load organizations. Please try again."),
    ).toBeInTheDocument();
    // No empty state and no org cards while erroring.
    expect(screen.queryByText("No organizations yet")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(mockRefetch).toHaveBeenCalledTimes(1);
  });
});
