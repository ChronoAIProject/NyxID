import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrmViewEditor } from "@/components/crm/workspace/view-editor";
import type { CrmSavedView } from "@/hooks/use-crm-views";
import { ApiError } from "@/lib/api-client";
import type { CrmViewEnvelope } from "@/lib/crm/views/schema";

const {
  mockValidate,
  mockCreate,
  mockUpdate,
  mockUseValidate,
  mockUseCreate,
  mockUseUpdate,
} = vi.hoisted(() => ({
  mockValidate: vi.fn(),
  mockCreate: vi.fn(),
  mockUpdate: vi.fn(),
  mockUseValidate: vi.fn(),
  mockUseCreate: vi.fn(),
  mockUseUpdate: vi.fn(),
}));

vi.mock("@/hooks/use-crm-views", () => ({
  useValidateCrmView: mockUseValidate,
  useCreateCrmView: mockUseCreate,
  useUpdateCrmView: mockUseUpdate,
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
        props: { content: "Pipeline", variant: "title" },
        children: [],
      },
    },
  },
};

const view: CrmSavedView = {
  id: "view-1",
  owner_id: "person-1",
  title: "Pipeline",
  revision: 7,
  envelope,
  valid: true,
  errors: [],
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T01:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  mockUseValidate.mockReturnValue({
    isPending: false,
    mutateAsync: mockValidate,
  });
  mockUseCreate.mockReturnValue({
    isPending: false,
    mutateAsync: mockCreate,
  });
  mockUseUpdate.mockReturnValue({
    isPending: false,
    mutateAsync: mockUpdate,
  });
  mockValidate.mockResolvedValue({ valid: true, errors: [] });
  mockUpdate.mockResolvedValue({ ...view, title: "Pipeline review" });
});

describe("CrmViewEditor", () => {
  it("validates with the server and shows returned path errors", async () => {
    mockValidate.mockResolvedValue({
      valid: false,
      errors: [
        {
          path: "spec.elements.chart.props.queryId",
          message: "The field was archived",
        },
      ],
    });
    render(
      <CrmViewEditor
        view={view}
        onSaved={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Validate" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "spec.elements.chart.props.queryId",
    );
    expect(mockValidate).toHaveBeenCalledWith(envelope);
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it("updates with expected_revision after validation", async () => {
    const onSaved = vi.fn();
    render(
      <CrmViewEditor
        view={view}
        onSaved={onSaved}
        onCancel={vi.fn()}
      />,
    );
    fireEvent.change(screen.getByLabelText("View name"), {
      target: { value: "Pipeline review" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Save view" }));

    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith({
        id: "view-1",
        title: "Pipeline review",
        envelope,
        expectedRevision: 7,
      }),
    );
    expect(onSaved).toHaveBeenCalledWith({
      ...view,
      title: "Pipeline review",
    });
  });

  it("preserves the draft and offers reload after a 409 conflict", async () => {
    mockUpdate.mockRejectedValue(
      new ApiError(409, {
        error: "crm_revision_conflict",
        error_code: 13_400,
        message: "The saved view changed",
      }),
    );
    const onReload = vi.fn();
    render(
      <CrmViewEditor
        view={view}
        onSaved={vi.fn()}
        onCancel={vi.fn()}
        onReload={onReload}
      />,
    );
    fireEvent.change(screen.getByLabelText("View name"), {
      target: { value: "My unsaved draft" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save view" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("changed after the editor opened");
    expect(screen.getByLabelText("View name")).toHaveValue("My unsaved draft");
    fireEvent.click(
      screen.getByRole("button", { name: "Reload current view" }),
    );
    expect(onReload).toHaveBeenCalledOnce();
  });
});
