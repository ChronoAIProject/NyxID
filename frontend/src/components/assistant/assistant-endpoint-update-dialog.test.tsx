import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AssistantEndpointUpdateDialog,
  type AssistantEndpointUpdateParams,
} from "./assistant-endpoint-update-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {
    readonly status: number;
    constructor(status: number, message = `HTTP ${String(status)}`) {
      super(message);
      this.status = status;
    }
  },
}));

const PARAMS = {
  endpointId: "00000000-0000-4000-8000-000000000001",
  label: "Renamed",
  endpointUrl: "https://api.example.com/v2",
} as const;

function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: PARAMS.endpointId,
    auto_connected: false,
    catalog_service_id: null,
    updated_at: "2026-08-20T10:00:00Z",
    ...overrides,
  };
}

function renderDialog(
  params: AssistantEndpointUpdateParams = PARAMS,
  onComplete = vi.fn(),
) {
  const onOpenChange = vi.fn();
  const rendered = render(
    <AssistantEndpointUpdateDialog
      open
      onOpenChange={onOpenChange}
      actionRequestId="action-endpoint-update"
      params={params}
      onComplete={onComplete}
    />,
  );
  return { ...rendered, onComplete, onOpenChange };
}

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantEndpointUpdateDialog", () => {
  it("fences double submission and reports only after secret-free evidence read-back", async () => {
    mockPost.mockResolvedValue({
      resource: { endpointId: PARAMS.endpointId },
      replayed: false,
    });
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockResolvedValueOnce(evidence({ updated_at: "2026-08-20T10:00:01Z" }));
    const { onComplete } = renderDialog();

    const submit = screen.getByRole("button", { name: "Update endpoint" });
    fireEvent.click(submit);
    fireEvent.click(submit);

    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith(
      "/assistant/actions/endpoints/update",
      {
        actionRequestId: "action-endpoint-update",
        endpointId: PARAMS.endpointId,
        label: "Renamed",
        endpointUrl: "https://api.example.com/v2",
      },
    );
    expect(mockGet).toHaveBeenNthCalledWith(
      1,
      `/endpoints/${PARAMS.endpointId}/authorization`,
    );
    expect(mockGet).toHaveBeenNthCalledWith(
      2,
      `/endpoints/${PARAMS.endpointId}/authorization`,
    );
    expect(
      await screen.findByText("Endpoint update verified."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Report endpoint" }));
    expect(onComplete).toHaveBeenCalledWith(PARAMS.endpointId);
  });

  it("refuses to finish when evidence carries a label", async () => {
    mockPost.mockResolvedValue({
      resource: { endpointId: PARAMS.endpointId },
      replayed: false,
    });
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockResolvedValueOnce({
        ...evidence({ updated_at: "2026-08-20T10:00:01Z" }),
        label: "Bearer nyxid_ag_abcdefghijklmnopqrst",
      });
    renderDialog();
    fireEvent.click(screen.getByRole("button", { name: "Update endpoint" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Report endpoint" })).toBeDisabled();
  });

  it("refuses to finish when the mutation timestamp does not advance", async () => {
    mockPost.mockResolvedValue({
      resource: { endpointId: PARAMS.endpointId },
      replayed: false,
    });
    mockGet
      .mockResolvedValueOnce(evidence())
      .mockResolvedValueOnce(evidence());
    renderDialog();
    fireEvent.click(screen.getByRole("button", { name: "Update endpoint" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "did not show the requested update",
    );
    expect(screen.getByRole("button", { name: "Report endpoint" })).toBeDisabled();
  });
});
