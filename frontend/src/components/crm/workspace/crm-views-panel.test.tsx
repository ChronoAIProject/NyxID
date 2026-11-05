import { renderCrmRoute } from "@/test/crm-router";
import { fireEvent, screen, waitFor } from "@testing-library/react";
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
  useDeleteCrmView: () => ({ isPending: false, mutateAsync: vi.fn() }),
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
    data: { views: [view], next_cursor: null },
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
  it("hides create and edit affordances for read access", async () => {
    mockObjects.mockReturnValue({ data: [{ access: "read" }] });
    await renderCrmRoute(<CrmViewsPanel ownerId="owner-1" />);

    expect(
      screen.queryByRole("button", { name: "New view" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Accounts/ }));
    expect(
      screen.queryByRole("button", { name: "Edit" }),
    ).not.toBeInTheDocument();
  });

  it("shows create and edit affordances for work access", async () => {
    mockObjects.mockReturnValue({ data: [{ access: "work" }] });
    await renderCrmRoute(<CrmViewsPanel ownerId="owner-1" />);

    expect(screen.getByRole("button", { name: "New view" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /Accounts/ }));
    expect(await screen.findByRole("button", { name: "Edit" })).toBeVisible();
  });
});

vi.mock("@/components/crm/workspace/renderer", () => ({
  CrmViewRenderer: ({
    recordId,
    onRecordChange,
  }: {
    recordId?: string;
    onRecordChange: (id?: string) => void;
  }) => (
    <div>
      <button onClick={() => onRecordChange("record-1")}>
        Open artifact record
      </button>
      {recordId ? (
        <div role="dialog">
          {recordId}
          <button onClick={() => onRecordChange(undefined)}>
            Close artifact record
          </button>
        </div>
      ) : null}
    </div>
  ),
}));
it("keeps saved-view record selection in router search through close, Back and Forward", async () => {
  mockObjects.mockReturnValue({ data: [{ access: "read" }] });
  const { router, history } = await renderCrmRoute(
    <CrmViewsPanel ownerId="owner-1" />,
    "/assistant/crm?owner=owner-1&view=view-1&panel=settings",
  );
  fireEvent.click(screen.getByText("Open artifact record"));
  await waitFor(() =>
    expect(router.state.location.search).toMatchObject({
      owner: "owner-1",
      view: "view-1",
      record: "record-1",
      panel: "settings",
    }),
  );
  fireEvent.click(screen.getByText("Close artifact record"));
  await waitFor(() =>
    expect(router.state.location.search.record).toBeUndefined(),
  );
  history.back();
  expect(await screen.findByRole("dialog")).toHaveTextContent("record-1");
  history.forward();
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
});
