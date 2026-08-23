import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { AssistantOrgActionDialog, type AssistantOrgAction } from "./assistant-org-action-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({ mockGet: vi.fn(), mockPost: vi.fn() }));
vi.mock("@/lib/api-client", () => {
  class MockApiError extends Error {
    readonly status: number;
    constructor(status: number, response: { message?: string }) {
      super(response.message ?? `HTTP ${String(status)}`);
      this.status = status;
    }
  }
  return { api: { get: mockGet, post: mockPost }, ApiError: MockApiError };
});

const ORG_ID = "00000000-0000-4000-8000-000000000041";
const MEMBER_ID = "00000000-0000-4000-8000-000000000042";
const USER_ID = "00000000-0000-4000-8000-000000000043";

function orgEvidence(overrides: Record<string, unknown> = {}) {
  return {
    id: ORG_ID,
    your_role: "admin",
    member_count: 1,
    is_primary: false,
    active_invite_count: 0,
    remote_credential_integrity_verification_opt_out: false,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function memberEvidence(overrides: Record<string, unknown> = {}) {
  return {
    membership_id: "00000000-0000-4000-8000-000000000044",
    user_id: MEMBER_ID,
    role: "viewer",
    scope_source: "inherit",
    allowed_service_ids: null,
    effective_allowed_service_ids: null,
    created_at: "2026-01-01T00:00:00Z",
    revoked_at: null,
    ...overrides,
  };
}

function renderDialog(action: AssistantOrgAction, params: Record<string, unknown>) {
  const onComplete = vi.fn();
  render(<AssistantOrgActionDialog open onOpenChange={vi.fn()} actionRequestId={`request-${action}`} action={action} params={params} onComplete={onComplete} />);
  return onComplete;
}

async function clickSubmit(destructive = false) {
  if (destructive) await userEvent.click(screen.getByRole("checkbox"));
  const button = screen.getByRole("button", { name: destructive ? "Confirm change" : "Continue" });
  fireEvent.click(button);
  fireEvent.click(button);
  await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
}

beforeEach(() => { mockGet.mockReset(); mockPost.mockReset(); });

describe("AssistantOrgActionDialog", () => {
  it("runs org.create and verifies the canonical projection", async () => {
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    mockGet.mockResolvedValue(orgEvidence());
    renderDialog("create", { displayName: "Acme" });
    await clickSubmit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/create", expect.objectContaining({ displayName: "Acme" }));
    expect(mockGet).toHaveBeenCalledWith(`/orgs/${ORG_ID}/authorization`);
  });

  it("runs org.update only after a live pre-read and observes a newer state", async () => {
    mockGet.mockResolvedValueOnce(orgEvidence()).mockResolvedValueOnce(orgEvidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    renderDialog("update", { orgId: ORG_ID, displayName: "Renamed" });
    await clickSubmit();
    expect(mockGet).toHaveBeenCalledTimes(2);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/update", expect.objectContaining({ orgId: ORG_ID, displayName: "Renamed" }));
  });

  it("runs org.delete only after a 200 pre-read, then accepts the causal 404", async () => {
    mockGet.mockResolvedValueOnce(orgEvidence()).mockRejectedValueOnce(new ApiError(404, { message: "not found" } as never));
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    const onComplete = renderDialog("delete", { orgId: ORG_ID });
    await clickSubmit(true);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/delete", expect.objectContaining({ confirmed: true }));
    expect(mockGet).toHaveBeenNthCalledWith(1, `/orgs/${ORG_ID}/authorization`);
    expect(mockGet).toHaveBeenNthCalledWith(2, `/orgs/${ORG_ID}/authorization`);
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(ORG_ID);
  });

  it("blocks org.delete when the canonical pre-read route is missing", async () => {
    mockGet.mockRejectedValue(new ApiError(404, { message: "not found" } as never));
    renderDialog("delete", { orgId: ORG_ID });
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Confirm change" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("runs org.member_add and proves the member count increased", async () => {
    mockGet.mockResolvedValueOnce(orgEvidence()).mockResolvedValueOnce(orgEvidence({ member_count: 2 }));
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    renderDialog("member_add", { orgId: ORG_ID, userId: USER_ID, role: "member" });
    await clickSubmit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/member-add", expect.objectContaining({ userId: USER_ID, role: "member" }));
  });

  it("runs org.member_remove and proves a terminal revoked membership", async () => {
    mockGet
      .mockResolvedValueOnce(orgEvidence({ member_count: 2 }))
      .mockResolvedValueOnce(memberEvidence())
      .mockResolvedValueOnce(orgEvidence({ member_count: 1 }))
      .mockResolvedValueOnce(memberEvidence({ revoked_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    renderDialog("member_remove", { orgId: ORG_ID, memberId: MEMBER_ID });
    await clickSubmit(true);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/member-remove", expect.objectContaining({ confirmed: true }));
    expect(mockGet).toHaveBeenLastCalledWith(`/orgs/${ORG_ID}/members/${MEMBER_ID}/authorization`);
  });

  it("runs org.member_update_role and proves the exact new role", async () => {
    mockGet
      .mockResolvedValueOnce(orgEvidence())
      .mockResolvedValueOnce(memberEvidence())
      .mockResolvedValueOnce(orgEvidence())
      .mockResolvedValueOnce(memberEvidence({ role: "admin" }));
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    renderDialog("member_update_role", { orgId: ORG_ID, memberId: MEMBER_ID, role: "admin" });
    await clickSubmit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/member-update-role", expect.objectContaining({ role: "admin", expectedRole: "viewer" }));
  });

  it("runs org.invite and proves the active invite count increased", async () => {
    mockGet.mockResolvedValueOnce(orgEvidence()).mockResolvedValueOnce(orgEvidence({ active_invite_count: 1 }));
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    renderDialog("invite", { orgId: ORG_ID, role: "viewer" });
    await clickSubmit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/invite", expect.objectContaining({ ttlHours: 24 }));
  });

  it("runs org.set_primary and proves the primary flag", async () => {
    mockGet.mockResolvedValueOnce(orgEvidence()).mockResolvedValueOnce(orgEvidence({ is_primary: true }));
    mockPost.mockResolvedValue({ resource: { orgId: ORG_ID }, replayed: false });
    renderDialog("set_primary", { orgId: ORG_ID });
    await clickSubmit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/org/set-primary", expect.objectContaining({ orgId: ORG_ID }));
  });
});
