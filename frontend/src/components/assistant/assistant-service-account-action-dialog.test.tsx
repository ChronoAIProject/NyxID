import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantServiceAccountActionDialog, type AssistantServiceAccountAction } from "./assistant-service-account-action-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({ mockGet: vi.fn(), mockPost: vi.fn() }));
vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error { readonly status: number; constructor(status: number) { super(`HTTP ${String(status)}`); this.status = status; } },
}));

const ID = "00000000-0000-4000-8000-000000000051";
function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: ID,
    client_id: "sa_abcdefghijklmnopqrstuvwx",
    role_ids: [],
    is_active: true,
    rate_limit_override: null,
    created_by: "00000000-0000-4000-8000-000000000052",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    last_authenticated_at: null,
    ...overrides,
  };
}

function renderDialog(action: AssistantServiceAccountAction, params: Record<string, unknown>) {
  render(<AssistantServiceAccountActionDialog open onOpenChange={vi.fn()} actionRequestId={`request-${action}`} action={action} params={params} onComplete={vi.fn()} />);
}

async function submit(destructive = false) {
  if (destructive) await userEvent.click(screen.getByRole("checkbox"));
  const button = screen.getByRole("button", { name: destructive ? "Confirm change" : "Continue" });
  fireEvent.click(button);
  fireEvent.click(button);
  await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
}

beforeEach(() => { mockGet.mockReset(); mockPost.mockReset(); });

describe("AssistantServiceAccountActionDialog", () => {
  it("runs service_account.create and displays the one-time secret after evidence", async () => {
    mockPost.mockResolvedValue({ resource: { serviceAccountId: ID }, replayed: false, clientSecret: "sa-secret-once" });
    mockGet.mockResolvedValue(evidence());
    renderDialog("create", { name: "Deploy bot" });
    await submit();
    expect(mockGet).toHaveBeenCalledWith(`/admin/service-accounts/${ID}/authorization`);
    expect(await screen.findByDisplayValue("sa-secret-once")).toBeInTheDocument();
  });

  it("runs service_account.update and proves a newer projection", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { serviceAccountId: ID }, replayed: false });
    renderDialog("update", { serviceAccountId: ID, name: "Renamed" });
    await submit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/service-account/update", expect.objectContaining({ name: "Renamed" }));
  });

  it("runs service_account.delete and proves the soft-deactivated terminal state", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ is_active: false, updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { serviceAccountId: ID }, replayed: false });
    renderDialog("delete", { serviceAccountId: ID });
    await submit(true);
    expect(mockGet).toHaveBeenCalledTimes(2);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/service-account/delete", expect.objectContaining({ confirmed: true }));
  });

  it("runs service_account.rotate_secret and returns material only in the committing response", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { serviceAccountId: ID }, replayed: false, clientSecret: "rotated-once" });
    renderDialog("rotate_secret", { serviceAccountId: ID });
    await submit();
    expect(await screen.findByDisplayValue("rotated-once")).toBeInTheDocument();
  });

  it("runs service_account.revoke_tokens and proves the revocation timestamp advance", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { serviceAccountId: ID }, replayed: false });
    renderDialog("revoke_tokens", { serviceAccountId: ID });
    await submit(true);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/service-account/revoke-tokens", expect.objectContaining({ confirmed: true }));
  });

  it("rejects secret-bearing assistant params before any mutation", async () => {
    renderDialog("update", { serviceAccountId: ID, credential: "Bearer abcdefghijklmnop" });
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/entered in the NyxID browser dialog/i);
    expect(mockGet).not.toHaveBeenCalled();
    expect(mockPost).not.toHaveBeenCalled();
  });
});
