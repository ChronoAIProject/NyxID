import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantServiceRouteDialog } from "./assistant-service-route-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error {
    readonly status: number;
    constructor(status: number) {
      super(`HTTP ${String(status)}`);
      this.status = status;
    }
  },
}));

const SERVICE_ID = "11111111-1111-4111-8111-111111111111";
const NODE_ID = "33333333-3333-4333-8333-333333333333";

beforeEach(() => {
  mockGet.mockReset();
  mockPost.mockReset();
});

describe("AssistantServiceRouteDialog", () => {
  it("sets node routing and verifies node_id on the evidence projection", async () => {
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    mockGet.mockResolvedValueOnce({
      id: SERVICE_ID,
      is_active: true,
      status: "active",
      connection_status: null,
      granted_scopes: null,
      last_authorized_at: null,
      node_id: NODE_ID,
      state_version: 1,
      updated_at: "2026-08-19T00:00:00Z",
      rotation_predecessor_id: null,
    });
    mockGet.mockResolvedValueOnce({
      id: SERVICE_ID,
      is_active: true,
      status: "active",
      connection_status: null,
      granted_scopes: null,
      last_authorized_at: null,
      node_id: NODE_ID,
      state_version: 2,
      updated_at: "2026-08-20T00:00:00Z",
      rotation_predecessor_id: null,
    });
    const onComplete = vi.fn();
    render(
      <AssistantServiceRouteDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-route"
        params={{ userServiceId: SERVICE_ID, viaNodeId: NODE_ID }}
        onComplete={onComplete}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Apply routing" }),
    );
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/services/route", {
      actionRequestId: "act-route",
      userServiceId: SERVICE_ID,
      viaNodeId: NODE_ID,
    });
    expect(
      await screen.findByText("Routing evidence verified."),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Report service" }),
    );
    expect(onComplete).toHaveBeenCalledWith(SERVICE_ID);
  });

  it("rejects stale evidence even when node_id already matches", async () => {
    mockPost.mockResolvedValue({
      resource: { userServiceId: SERVICE_ID },
      replayed: false,
    });
    const stale = {
      id: SERVICE_ID,
      is_active: true,
      status: "active",
      connection_status: null,
      granted_scopes: null,
      last_authorized_at: null,
      node_id: NODE_ID,
      state_version: 1,
      updated_at: "2026-08-19T00:00:00Z",
      rotation_predecessor_id: null,
    };
    mockGet.mockResolvedValue(stale);
    render(
      <AssistantServiceRouteDialog
        open
        onOpenChange={vi.fn()}
        actionRequestId="act-route-stale"
        params={{ userServiceId: SERVICE_ID, viaNodeId: NODE_ID }}
        onComplete={vi.fn()}
      />,
    );

    await userEvent.click(
      screen.getByRole("button", { name: "Apply routing" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "expected state advance",
    );
    expect(
      screen.getByRole("button", { name: "Report service" }),
    ).toBeDisabled();
  });
});
