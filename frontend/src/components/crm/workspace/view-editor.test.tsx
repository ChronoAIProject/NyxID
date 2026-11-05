import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrmViewEditor } from "@/components/crm/workspace/view-editor";
import type { CrmSavedView } from "@/hooks/use-crm-views";
import { ApiError } from "@/lib/api-client";
import type { CrmViewEnvelope } from "@/lib/crm/views/schema";

const {
  mockGenerate,
  mockValidate,
  mockCreate,
  mockUpdate,
  mockUseValidate,
  mockUseCreate,
  mockUseUpdate,
} = vi.hoisted(() => ({
  mockGenerate: vi.fn(),
  mockValidate: vi.fn(),
  mockCreate: vi.fn(),
  mockUpdate: vi.fn(),
  mockUseValidate: vi.fn(),
  mockUseCreate: vi.fn(),
  mockUseUpdate: vi.fn(),
}));

vi.mock("@/hooks/use-crm-views", () => ({
  useGenerateCrmView: () => ({ mutateAsync: mockGenerate, isPending: false }),
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
  vi.resetAllMocks();
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
  mockGenerate.mockResolvedValue({
    envelope,
    valid: true,
    errors: [],
    repaired: false,
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
    render(<CrmViewEditor view={view} onSaved={vi.fn()} onCancel={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "Validate" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "spec.elements.chart.props.queryId",
    );
    expect(mockValidate).toHaveBeenCalledWith(envelope);
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it("updates with expected_revision after validation", async () => {
    const onSaved = vi.fn();
    render(<CrmViewEditor view={view} onSaved={onSaved} onCancel={vi.fn()} />);
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
        idempotencyKey: expect.any(String),
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

  it("reuses the action key when an unchanged draft retries an uncertain save", async () => {
    mockUpdate
      .mockRejectedValueOnce(new Error("Network unavailable"))
      .mockResolvedValueOnce({ ...view, title: "Retry draft" });
    render(<CrmViewEditor view={view} onSaved={vi.fn()} onCancel={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("View name"), {
      target: { value: "Retry draft" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Save view" }));
    await screen.findByText("Network unavailable");
    const firstKey = mockUpdate.mock.calls[0]?.[0].idempotencyKey as string;
    fireEvent.click(screen.getByRole("button", { name: "Save view" }));

    await waitFor(() => expect(mockUpdate).toHaveBeenCalledTimes(2));
    expect(mockUpdate.mock.calls[1]?.[0].idempotencyKey).toBe(firstKey);
  });
});

it("retains a dirty baseline across live revisions and advances pristine editors", async () => {
  const props = { onSaved: vi.fn(), onCancel: vi.fn() };
  const { rerender } = render(<CrmViewEditor {...props} view={view} />);
  rerender(
    <CrmViewEditor
      {...props}
      view={{ ...view, revision: 8, title: "Fresh title" }}
    />,
  );
  await waitFor(() =>
    expect(screen.getByLabelText("View name")).toHaveValue("Fresh title"),
  );
  fireEvent.change(screen.getByLabelText("View name"), {
    target: { value: "My draft" },
  });
  rerender(
    <CrmViewEditor
      {...props}
      view={{ ...view, revision: 9, title: "Other change" }}
    />,
  );
  expect(screen.getByLabelText("View name")).toHaveValue("My draft");
  fireEvent.click(screen.getByRole("button", { name: "Save view" }));
  await waitFor(() =>
    expect(mockUpdate).toHaveBeenCalledWith(
      expect.objectContaining({ title: "My draft", expectedRevision: 8 }),
    ),
  );
});

it("reuses generation identity only for unchanged object/prompt/draft and never autosaves", async () => {
  mockGenerate.mockRejectedValue(new Error("Generation uncertain"));
  render(<CrmViewEditor view={view} onSaved={vi.fn()} onCancel={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Describe a workspace"), {
    target: { value: "Company directory" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Generate draft" }));
  await screen.findByText("Generation uncertain");
  const first = mockGenerate.mock.calls[0]?.[0].idempotencyKey;
  fireEvent.click(
    screen.getByRole("button", { name: "Retry same generation" }),
  );
  await waitFor(() => expect(mockGenerate).toHaveBeenCalledTimes(2));
  expect(mockGenerate.mock.calls[1]?.[0].idempotencyKey).toBe(first);
  fireEvent.change(screen.getByLabelText("JSON envelope"), {
    target: { value: JSON.stringify(envelope) },
  });
  mockGenerate.mockResolvedValueOnce({
    envelope,
    valid: true,
    repaired: true,
    errors: [],
  });
  fireEvent.click(screen.getByRole("button", { name: "Generate draft" }));
  await waitFor(() => expect(mockGenerate).toHaveBeenCalledTimes(3));
  expect(mockGenerate.mock.calls[2]?.[0].idempotencyKey).not.toBe(first);
  expect(mockCreate).not.toHaveBeenCalled();
  expect(mockUpdate).not.toHaveBeenCalled();
  expect(await screen.findByText("View is valid.")).toBeVisible();
});

it("does not save an envelope when validation completes after the draft changed", async () => {
  let finish!: (result: { valid: boolean; errors: [] }) => void;
  mockValidate.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  render(<CrmViewEditor view={view} onSaved={vi.fn()} onCancel={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("View name"), {
    target: { value: "First" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save view" }));
  fireEvent.change(screen.getByLabelText("View name"), {
    target: { value: "Second" },
  });
  finish({ valid: true, errors: [] });
  await waitFor(() => expect(mockValidate).toHaveBeenCalledOnce());
  expect(mockUpdate).not.toHaveBeenCalled();
});

it("displays top-level server validation paths after schema drift on save", async () => {
  mockUpdate.mockRejectedValue(
    new ApiError(400, {
      error: "crm_view_invalid",
      error_code: 13403,
      message: "Invalid view",
      errors: [
        {
          path: "spec.elements.main.props.columns",
          message: "Displayed fields must be selected",
        },
      ],
    }),
  );
  render(<CrmViewEditor view={view} onSaved={vi.fn()} onCancel={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("View name"), {
    target: { value: "Changed" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save view" }));
  expect(
    await screen.findByText(/spec.elements.main.props.columns/),
  ).toBeVisible();
});

it("shows a new generation action after a failed request payload changes", async () => {
  mockGenerate.mockRejectedValue(new Error("Uncertain generation"));
  render(<CrmViewEditor view={view} onSaved={vi.fn()} onCancel={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Describe a workspace"), {
    target: { value: "Directory" },
  });
  fireEvent.click(screen.getByText("Generate draft"));
  await screen.findByText("Retry same generation");
  const first = mockGenerate.mock.calls[0]![0];
  fireEvent.change(screen.getByLabelText("JSON envelope"), {
    target: { value: JSON.stringify({ ...envelope, queries: {} }) + " " },
  });
  expect(screen.queryByText("Retry same generation")).not.toBeInTheDocument();
  expect(
    screen.queryByText(/Retrying keeps the same request identity/),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByText("Generate draft"));
  await waitFor(() => expect(mockGenerate).toHaveBeenCalledTimes(2));
  expect(mockGenerate.mock.calls[1]![0].idempotencyKey).not.toBe(
    first.idempotencyKey,
  );
});
