import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantDeveloperAppActionDialog, type AssistantDeveloperAppAction } from "./assistant-developer-app-action-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({ mockGet: vi.fn(), mockPost: vi.fn() }));
vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error { readonly status: number; constructor(status: number) { super(`HTTP ${String(status)}`); this.status = status; } },
}));

const ID = "00000000-0000-4000-8000-000000000061";
function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: ID,
    broker_capability_enabled: false,
    connection_webhook_enabled: false,
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function renderDialog(action: AssistantDeveloperAppAction, params: Record<string, unknown>) {
  render(<AssistantDeveloperAppActionDialog open onOpenChange={vi.fn()} actionRequestId={`request-${action}`} action={action} params={params} onComplete={vi.fn()} />);
}

async function submit(destructive = false) {
  if (destructive) await userEvent.click(screen.getByRole("checkbox"));
  const button = screen.getByRole("button", { name: destructive ? "Delete app" : "Continue" });
  fireEvent.click(button);
  fireEvent.click(button);
  await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
}

beforeEach(() => { mockGet.mockReset(); mockPost.mockReset(); });

describe("AssistantDeveloperAppActionDialog", () => {
  it("runs developer_app.create and displays its one-time secret", async () => {
    mockPost.mockResolvedValue({ resource: { clientId: ID }, replayed: false, clientSecret: "client-secret-once" });
    mockGet.mockResolvedValue(evidence());
    renderDialog("create", { name: "My app", redirectUris: ["https://app.example/callback"] });
    await submit();
    expect(mockGet).toHaveBeenCalledWith(`/developer/oauth-clients/${ID}/authorization`);
    expect(await screen.findByDisplayValue("client-secret-once")).toBeInTheDocument();
  });

  it("runs developer_app.update and proves a newer projection", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { clientId: ID }, replayed: false });
    renderDialog("update", { clientId: ID, name: "Renamed", redirectUris: ["https://app.example/callback"] });
    await submit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/developer-app/update", expect.objectContaining({ clientId: ID, name: "Renamed" }));
  });

  it("runs developer_app.delete and proves the soft-deactivated terminal state", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ is_active: false, updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { clientId: ID }, replayed: false });
    renderDialog("delete", { clientId: ID });
    await submit(true);
    expect(mockGet).toHaveBeenCalledTimes(2);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/developer-app/delete", expect.objectContaining({ confirmed: true }));
  });

  it("runs developer_app.rotate_secret and keeps the secret in the browser response", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { clientId: ID }, replayed: false, clientSecret: "rotated-client-secret" });
    renderDialog("rotate_secret", { clientId: ID });
    await submit();
    expect(await screen.findByDisplayValue("rotated-client-secret")).toBeInTheDocument();
  });
});
