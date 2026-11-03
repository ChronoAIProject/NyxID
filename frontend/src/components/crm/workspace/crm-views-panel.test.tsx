import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrmViewsPanel } from "@/components/crm/workspace";

const { mockObjects, mockViews, mockView } = vi.hoisted(() => ({
  mockObjects: vi.fn(),
  mockViews: vi.fn(),
  mockView: vi.fn(),
}));

vi.mock("@/hooks/use-crm", () => ({
  useCrmObjects: mockObjects,
}));

vi.mock("@/hooks/use-crm-views", () => ({
  useCrmViews: mockViews,
  useCrmView: mockView,
}));

const envelope = {
  formatVersion: 1,
  renderer: "json-render",
  rendererSpecVersion: "nyxid-crm-v1",
  queries: {},
  spec: {
    root: "main",
    elements: {
      main: {
        type: "Text",
        props: { content: "Saved content", variant: "body" },
        children: [],
      },
    },
  },
};

const view = {
  id: "view-1",
  owner_id: "owner-1",
  title: "Accounts",
  revision: 1,
  envelope,
  valid: true,
  errors: [],
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  mockViews.mockReturnValue({
    isPending: false,
    isError: false,
    isFetching: false,
    data: [view],
    refetch: vi.fn(),
  });
  mockView.mockImplementation((id: string | undefined) => ({
    isPending: false,
    isError: false,
    data: id ? view : undefined,
    refetch: vi.fn(),
  }));
});

describe("CrmViewsPanel access", () => {
  it("hides create and edit affordances for read access", () => {
    mockObjects.mockReturnValue({ data: [{ access: "read" }] });
    render(<CrmViewsPanel ownerId="owner-1" />);

    expect(
      screen.queryByRole("button", { name: "New view" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Accounts/ }));
    expect(
      screen.queryByRole("button", { name: "Edit" }),
    ).not.toBeInTheDocument();
  });

  it("shows create and edit affordances for work access", () => {
    mockObjects.mockReturnValue({ data: [{ access: "work" }] });
    render(<CrmViewsPanel ownerId="owner-1" />);

    expect(screen.getByRole("button", { name: "New view" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /Accounts/ }));
    expect(screen.getByRole("button", { name: "Edit" })).toBeVisible();
  });
});
