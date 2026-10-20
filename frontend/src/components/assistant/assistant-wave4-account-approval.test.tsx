import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantAccountDeleteDialog } from "./assistant-account-delete-dialog";
import { AssistantAccountMfaSetupDialog } from "./assistant-account-mfa-setup-dialog";
import { AssistantAccountProfileUpdateDialog } from "./assistant-account-profile-update-dialog";
import { AssistantAccountRevokeConsentDialog } from "./assistant-account-revoke-consent-dialog";
import { AssistantApprovalConfigureDialog } from "./assistant-approval-configure-dialog";
import { AssistantApprovalDisableDialog } from "./assistant-approval-disable-dialog";
import { AssistantApprovalRevokeGrantDialog } from "./assistant-approval-revoke-grant-dialog";

const IDS = {
  user: "00000000-0000-4000-8000-000000000001",
  client: "00000000-0000-4000-8000-000000000002",
  service: "00000000-0000-4000-8000-000000000003",
  grant: "00000000-0000-4000-8000-000000000004",
  factor: "00000000-0000-4000-8000-000000000005",
};

const { mockGet, mockPost, mockApiClient, mockSetUser, MockApiError } = vi.hoisted(() => {
  class HoistedApiError extends Error {
    readonly status: number;
    constructor(status: number, message = `HTTP ${String(status)}`) { super(message); this.status = status; }
  }
  return { mockGet: vi.fn(), mockPost: vi.fn(), mockApiClient: vi.fn(), mockSetUser: vi.fn(), MockApiError: HoistedApiError };
});
vi.mock("@/lib/api-client", () => ({ api: { get: mockGet, post: mockPost }, apiClient: mockApiClient, ApiError: MockApiError }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: { getState: () => ({ setUser: mockSetUser }) } }));

function renderDialog(element: ReactNode) { return render(element); }
function accountEvidence(overrides: Record<string, unknown> = {}) { return { id: IDS.user, is_active: true, mfa_enabled: false, display_name_length: 7, avatar_url_length: null, updated_at: "2026-01-01T00:00:00.000Z", ...overrides }; }
function configEvidence(overrides: Record<string, unknown> = {}) { return { service_id: IDS.service, approval_required: true, approval_mode: "per_request", rule_count: 0, default_effect: null, updated_at: "2026-01-01T00:00:00.000Z", ...overrides }; }

beforeEach(() => { mockGet.mockReset(); mockPost.mockReset(); mockSetUser.mockReset(); mockApiClient.mockReset(); mockApiClient.mockImplementation((endpoint: string) => mockGet(endpoint)); });

describe("Wave 4 account journeys", () => {
  it("discloses all managed Machine data and retained records in the actual account deletion dialog", () => {
    renderDialog(<AssistantAccountDeleteDialog open onOpenChange={vi.fn()} actionRequestId="delete-disclosure" onComplete={vi.fn()} />);
    const dialog = screen.getByRole("dialog", { name: "Delete account" });
    expect(dialog).toHaveTextContent(/all (?:of your )?managed Machines/i);
    expect(dialog).toHaveTextContent(/paused \/workspace disks/i);
    expect(dialog).toHaveTextContent(/temporary (?:files|data)/i);
    expect(dialog).toHaveTextContent(/irreversible/i);
    expect(dialog).toHaveTextContent(/no snapshots/i);
    expect(dialog).toHaveTextContent(/no recovery or restore/i);
    expect(dialog).toHaveTextContent(/at most 1 hour after a job ends/i);
    expect(dialog).toHaveTextContent(/30 days after the job ends/i);
    expect(dialog).toHaveTextContent(/90 days after completion/i);
    expect(dialog).toHaveTextContent(/audit and financial records/i);
    expect(dialog).toHaveTextContent(/cleanup pending/i);
    expect(screen.getByRole("button", { name: "Delete account" })).toBeDisabled();
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("updates a profile only after the projection advances", async () => {
    mockGet.mockResolvedValueOnce(accountEvidence()).mockResolvedValueOnce(accountEvidence({ display_name_length: 11, updated_at: "2026-01-01T00:00:01.000Z" }));
    mockPost.mockResolvedValue({ resource: { userId: IDS.user }, replayed: false });
    const onComplete = vi.fn();
    renderDialog(<AssistantAccountProfileUpdateDialog open onOpenChange={vi.fn()} actionRequestId="profile-1" params={{ displayName: "new profile" }} onComplete={onComplete} />);
    await userEvent.click(screen.getByRole("button", { name: "Update profile" }));
    await waitFor(() => expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/account/profile-update", expect.objectContaining({ actionRequestId: "profile-1" })));
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(IDS.user);
  });

  it("rejects secret-shaped profile material before mutation", async () => {
    renderDialog(<AssistantAccountProfileUpdateDialog open onOpenChange={vi.fn()} actionRequestId="profile-secret" params={{ displayName: "Bearer nyxid_ag_1234567890123456" }} onComplete={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "Update profile" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(mockPost).not.toHaveBeenCalled();
  });

  it("requires body-free absence evidence for consent revoke", async () => {
    mockPost.mockResolvedValue({ resource: { userId: IDS.user }, replayed: false });
    mockGet.mockResolvedValueOnce({ id: IDS.client, granted_scopes: ["openid"] }).mockRejectedValueOnce(new MockApiError(404));
    const onComplete = vi.fn();
    renderDialog(<AssistantAccountRevokeConsentDialog open onOpenChange={vi.fn()} actionRequestId="consent-1" params={{ clientId: IDS.client }} onComplete={onComplete} />);
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Revoke consent" }));
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/account/revoke-consent", {
      actionRequestId: "consent-1",
      clientId: IDS.client,
      confirmed: true,
    });
    expect(mockGet).toHaveBeenNthCalledWith(1, `/users/me/consents/${IDS.client}/authorization`);
    expect(mockGet).toHaveBeenNthCalledWith(2, `/users/me/consents/${IDS.client}/authorization`);
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(IDS.user);
  });

  it("requires the account email and server policy acknowledgement on every destructive delete", async () => {
    mockGet.mockResolvedValueOnce({ email: "owner@example.com" }).mockResolvedValueOnce(accountEvidence());
    mockPost.mockResolvedValue({ resource: { userId: IDS.user }, replayed: false,
      erasure: { status: "accepted", accepted_at: "2026-10-19T00:00:00Z", completed_at: null, domains: [] } });
    renderDialog(<AssistantAccountDeleteDialog open onOpenChange={vi.fn()} actionRequestId="delete-1" onComplete={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Account email"), "owner@example.com");
    await userEvent.click(screen.getByRole("button", { name: "Delete account" }));
    expect(await screen.findByRole("button", { name: "Continue" })).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toHaveAccessibleName("Account deletion accepted");
    expect(mockGet).toHaveBeenNthCalledWith(2, "/users/me/authorization");
    expect(mockGet).toHaveBeenCalledTimes(2);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/account/delete", {
      actionRequestId: "delete-1", confirmEmail: "owner@example.com", deletionPolicyVersion: "machine-data-provider-revocation-v2",
    });
  });

  it("reports durable acceptance without inferring erasure from a revoked session and clears the session on Continue", async () => {
    mockGet.mockResolvedValueOnce({ email: "owner@example.com" }).mockResolvedValueOnce(accountEvidence());
    mockPost.mockResolvedValue({ resource: { userId: IDS.user }, replayed: false,
      erasure: { status: "pending", accepted_at: "2026-10-19T00:00:00Z", completed_at: null,
        domains: [{ domain: "provider_credentials", status: "pending", reason: null, completed_at: null }] } });
    const onComplete = vi.fn();
    renderDialog(<AssistantAccountDeleteDialog open onOpenChange={vi.fn()} actionRequestId="delete-revoked-session" onComplete={onComplete} />);
    await userEvent.type(screen.getByLabelText("Account email"), "owner@example.com");
    await userEvent.click(screen.getByRole("button", { name: "Delete account" }));
    expect(await screen.findByRole("button", { name: "Continue" })).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toHaveTextContent("cleanup continues until verified");
    expect(mockApiClient).not.toHaveBeenCalled();
    expect(mockGet).toHaveBeenCalledTimes(2);
    expect(mockSetUser).not.toHaveBeenCalled();
    expect(onComplete).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(onComplete).toHaveBeenCalledWith(IDS.user);
    expect(mockSetUser).toHaveBeenCalledExactlyOnceWith(null);
  });

  it("rejects a legacy success response without durable erasure acceptance", async () => {
    mockGet.mockResolvedValueOnce({ email: "owner@example.com" }).mockResolvedValueOnce(accountEvidence());
    mockPost.mockResolvedValue({ resource: { userId: IDS.user }, replayed: false });
    const onComplete = vi.fn();
    renderDialog(<AssistantAccountDeleteDialog open onOpenChange={vi.fn()} actionRequestId="delete-unproven" onComplete={onComplete} />);
    await userEvent.type(screen.getByLabelText("Account email"), "owner@example.com");
    await userEvent.click(screen.getByRole("button", { name: "Delete account" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument();
    expect(onComplete).not.toHaveBeenCalled();
    expect(mockSetUser).not.toHaveBeenCalled();
  });

  it.each([401, 403, 500])("does not turn a rejected deletion mutation (%s) into account deletion success", async (status) => {
    mockGet.mockResolvedValueOnce({ email: "owner@example.com" }).mockResolvedValueOnce(accountEvidence());
    mockPost.mockRejectedValue(new MockApiError(status));
    const onComplete = vi.fn();
    renderDialog(<AssistantAccountDeleteDialog open onOpenChange={vi.fn()} actionRequestId={`delete-rejected-${String(status)}`} onComplete={onComplete} />);
    await userEvent.type(screen.getByLabelText("Account email"), "owner@example.com");
    await userEvent.click(screen.getByRole("button", { name: "Delete account" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(`HTTP ${String(status)}`);
    expect(screen.queryByRole("button", { name: "Continue" })).not.toBeInTheDocument();
    expect(mockGet).toHaveBeenCalledTimes(2);
    expect(mockApiClient).not.toHaveBeenCalled();
    expect(onComplete).not.toHaveBeenCalled();
  });

  it("keeps MFA material in the browser and reports only the account id", async () => {
    mockPost.mockResolvedValueOnce({ resource: { userId: IDS.user }, stage: "start", factorId: IDS.factor, setupValue: "JBSWY3DPEHPK3PXP", qrCodeUrl: "otpauth://totp/NyxID:test", recoveryValues: null, replayed: false, oneTimeMaterial: "delivered" }).mockResolvedValueOnce({ resource: { userId: IDS.user }, stage: "confirm", factorId: IDS.factor, recoveryValues: ["recovery-1"], replayed: false, oneTimeMaterial: "delivered" });
    mockGet.mockResolvedValue({ id: IDS.user, mfa_enabled: true });
    const onComplete = vi.fn();
    renderDialog(<AssistantAccountMfaSetupDialog open onOpenChange={vi.fn()} actionRequestId="mfa-1" onComplete={onComplete} />);
    await userEvent.click(screen.getByRole("button", { name: "Start setup" }));
    await userEvent.type(await screen.findByLabelText("Authenticator code"), "123456");
    await userEvent.click(screen.getByRole("button", { name: "Enable MFA" }));
    await userEvent.click(await screen.findByRole("button", { name: "I have saved them" }));
    expect(onComplete).toHaveBeenCalledWith(IDS.user);
    // noUncheckedIndexedAccess: index access is `T | undefined`, so assert the
    // call exists before asserting on its body -- otherwise the guarantee this
    // test claims (no setup value is ever posted) could vacuously pass.
    const confirmCall = mockPost.mock.calls[1];
    expect(confirmCall).toBeDefined();
    expect(confirmCall?.[1]).not.toHaveProperty("setupValue");
  });
});

describe("Wave 4 approval journeys", () => {
  it("configures a service and verifies the typed policy projection", async () => {
    mockGet.mockRejectedValueOnce(new MockApiError(404)).mockResolvedValueOnce(configEvidence());
    mockPost.mockResolvedValue({ resource: { serviceId: IDS.service }, replayed: false });
    const onComplete = vi.fn();
    renderDialog(<AssistantApprovalConfigureDialog open onOpenChange={vi.fn()} actionRequestId="config-1" params={{ serviceId: IDS.service }} onComplete={onComplete} />);
    await userEvent.click(screen.getByRole("button", { name: "Save policy" }));
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(IDS.service);
  });

  it("never disables approvals without a fresh destructive confirmation", async () => {
    mockGet.mockResolvedValueOnce(configEvidence()).mockResolvedValueOnce(configEvidence({ approval_required: false, updated_at: "2026-01-01T00:00:01.000Z" }));
    mockPost.mockResolvedValue({ resource: { serviceId: IDS.service }, replayed: false });
    renderDialog(<AssistantApprovalDisableDialog open onOpenChange={vi.fn()} actionRequestId="disable-1" params={{ serviceId: IDS.service }} onComplete={vi.fn()} />);
    const disable = screen.getByRole("button", { name: "Disable approvals" });
    expect(disable).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(disable);
    expect(await screen.findByRole("button", { name: "Done" })).toBeInTheDocument();
  });

  it("requires active grant evidence before and revoked evidence after", async () => {
    mockGet.mockResolvedValueOnce({ id: IDS.grant, revoked: false }).mockResolvedValueOnce({ id: IDS.grant, revoked: true });
    mockPost.mockResolvedValue({ resource: { grantId: IDS.grant }, replayed: false });
    const onComplete = vi.fn();
    renderDialog(<AssistantApprovalRevokeGrantDialog open onOpenChange={vi.fn()} actionRequestId="grant-1" params={{ grantId: IDS.grant }} onComplete={onComplete} />);
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Revoke grant" }));
    await userEvent.click(await screen.findByRole("button", { name: "Done" }));
    expect(onComplete).toHaveBeenCalledWith(IDS.grant);
  });
});
