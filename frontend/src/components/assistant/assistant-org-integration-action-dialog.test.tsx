import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantOrgIntegrationActionDialog } from "./assistant-org-integration-action-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({ mockGet: vi.fn(), mockPost: vi.fn() }));
vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error { readonly status: number; constructor(status: number) { super(`HTTP ${String(status)}`); this.status = status; } },
}));

const EXTERNAL_ID = "00000000-0000-4000-8000-000000000081";
const SERVICE_ID = "00000000-0000-4000-8000-000000000082";

beforeEach(() => { mockGet.mockReset(); mockPost.mockReset(); });

describe("AssistantOrgIntegrationActionDialog", () => {
  it("runs external_key.add_gcp_service_account and reads the canonical external-key projection", async () => {
    mockPost.mockResolvedValue({ resource: { externalKeyId: EXTERNAL_ID }, replayed: false });
    mockGet.mockResolvedValue({
      id: EXTERNAL_ID,
      credential_type: "gcp_service_account",
      status: "active",
      expires_at: null,
      last_used_at: null,
      updated_at: "2026-01-01T00:00:00Z",
    });
    render(<AssistantOrgIntegrationActionDialog open onOpenChange={vi.fn()} actionRequestId="request-gcp" action="external_key.add_gcp_service_account" params={{ label: "GCP prod" }} onComplete={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Service-account JSON"), {
      target: { value: '{"type":"service_account","private_key":"private"}' },
    });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/external-key/add-gcp-service-account", expect.objectContaining({ keyJson: expect.stringContaining("private_key") }));
    expect(mockGet).toHaveBeenCalledWith(`/api-keys/external/${EXTERNAL_ID}/authorization`);
  });

  it("runs openclaw.connect with browser-only credential entry and canonical service evidence", async () => {
    mockPost.mockResolvedValue({ resource: { userServiceId: SERVICE_ID }, replayed: false });
    mockGet.mockResolvedValue({
      id: SERVICE_ID,
      api_key_id: "00000000-0000-4000-8000-000000000083",
      is_active: true,
      status: "active",
      connection_status: "connected",
      granted_scopes: null,
      last_authorized_at: null,
      node_id: null,
      state_version: 1,
      updated_at: "2026-01-01T00:00:00Z",
      rotation_predecessor_id: null,
    });
    render(<AssistantOrgIntegrationActionDialog open onOpenChange={vi.fn()} actionRequestId="request-openclaw" action="openclaw.connect" params={{ gatewayUrl: "https://openclaw.example" }} onComplete={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Bearer credential"), "browser-only-bearer");
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/openclaw/connect", expect.objectContaining({ gatewayUrl: "https://openclaw.example", credential: "browser-only-bearer" }));
    expect(mockGet).toHaveBeenCalledWith(`/keys/${SERVICE_ID}/authorization`);
  });
});
