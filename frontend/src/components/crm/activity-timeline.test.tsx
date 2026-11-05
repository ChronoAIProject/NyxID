import type { ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { CrmActivity } from "@/schemas/crm";
import { CrmActivityTimeline } from "./activity-timeline";

const { history, addNote } = vi.hoisted(() => ({
  history: vi.fn(),
  addNote: vi.fn(),
}));

vi.mock("@/hooks/use-crm", () => ({
  useCrmActivityHistory: history,
  useAddCrmNote: addNote,
}));
vi.mock("@/components/crm/people-picker", () => ({
  CrmPeopleLabels: ({ children }: { children: ReactNode }) => children,
  CrmPersonName: () => "Avery",
}));

beforeEach(() => {
  vi.resetAllMocks();
  addNote.mockReturnValue({ isPending: false, isError: false });
});

it.each([
  { fields: ["stage"], accompanying: false },
  { fields: ["amount", "stage"], accompanying: true },
])(
  "renders one stage activity with accompanying fields=$accompanying",
  ({ fields, accompanying }) => {
    const activity: CrmActivity = {
      id: "event",
      record_id: "record",
      object_key: "opportunity",
      kind: "stage_changed",
      actor: {
        kind: "human",
        user_id: "person",
        agent_id: null,
        api_key_id: null,
        source: null,
      },
      summary: { changed_fields: fields, from: "new", to: "qualified" },
      body: undefined,
      created_at: "2026-11-04T00:00:00Z",
    };
    history.mockReturnValue({
      data: { pages: [{ activities: [activity], next_cursor: null }] },
      isPending: false,
      isError: false,
      hasNextPage: false,
    });

    render(
      <CrmActivityTimeline
        recordId="record"
        ownerId="owner"
        canWrite={false}
      />,
    );

    expect(screen.getAllByRole("listitem")).toHaveLength(1);
    expect(
      screen.getByText("Stage changed from new to qualified"),
    ).toBeVisible();
    if (accompanying) {
      expect(screen.getByText("Changed fields: amount, stage")).toBeVisible();
    } else {
      expect(screen.queryByText(/Changed fields:/)).not.toBeInTheDocument();
    }
  },
);
