import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantNotificationsActionDialog, type AssistantNotificationsAction } from "./assistant-notifications-action-dialog";

const { mockGet, mockPost } = vi.hoisted(() => ({ mockGet: vi.fn(), mockPost: vi.fn() }));
vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
  ApiError: class ApiError extends Error { readonly status: number; constructor(status: number) { super(`HTTP ${String(status)}`); this.status = status; } },
}));

const ID = "00000000-0000-4000-8000-000000000071";
function evidence(overrides: Record<string, unknown> = {}) {
  return {
    id: ID,
    telegram_connected: false,
    telegram_link_pending: false,
    telegram_enabled: false,
    approval_required: false,
    approval_timeout_secs: 30,
    grant_expiry_days: 30,
    push_enabled: false,
    push_device_count: 0,
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function renderDialog(action: AssistantNotificationsAction) {
  render(<AssistantNotificationsActionDialog open onOpenChange={vi.fn()} actionRequestId={`request-${action}`} action={action} params={{}} onComplete={vi.fn()} />);
}

async function submit(destructive = false) {
  if (destructive) await userEvent.click(screen.getByRole("checkbox"));
  await waitFor(() => expect(screen.getByRole("button", { name: destructive ? "Disconnect" : "Continue" })).toBeEnabled());
  const button = screen.getByRole("button", { name: destructive ? "Disconnect" : "Continue" });
  fireEvent.click(button);
  fireEvent.click(button);
  await waitFor(() => expect(mockPost).toHaveBeenCalledTimes(1));
}

beforeEach(() => { mockGet.mockReset(); mockPost.mockReset(); });

describe("AssistantNotificationsActionDialog", () => {
  it("runs notifications.update with browser-collected settings and exact read-back", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { bindingId: ID }, replayed: false });
    renderDialog("update");
    await submit();
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/notifications/update", {
      actionRequestId: "request-update",
      telegramEnabled: false,
      approvalRequired: false,
      approvalTimeoutSecs: 30,
      grantExpiryDays: 30,
      pushEnabled: false,
    });
  });

  it("runs notifications.telegram_link, proves pending state, and displays the code once", async () => {
    mockGet.mockResolvedValueOnce(evidence()).mockResolvedValueOnce(evidence({ telegram_link_pending: true, updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { bindingId: ID }, replayed: false, linkCode: "NYXID-ABC12345", botUsername: "NyxIDBot", expiresInSecs: 300 });
    renderDialog("telegram_link");
    await submit();
    expect(await screen.findByDisplayValue("NYXID-ABC12345")).toBeInTheDocument();
    expect(mockGet).toHaveBeenNthCalledWith(1, "/notifications/settings/authorization");
    expect(mockGet).toHaveBeenNthCalledWith(2, "/notifications/settings/authorization");
  });

  it("runs notifications.telegram_disconnect and proves the terminal state", async () => {
    mockGet
      .mockResolvedValueOnce(evidence({ telegram_connected: true, telegram_enabled: true }))
      .mockResolvedValueOnce(evidence({ updated_at: "2026-01-01T00:00:01Z" }));
    mockPost.mockResolvedValue({ resource: { bindingId: ID }, replayed: false });
    renderDialog("telegram_disconnect");
    await submit(true);
    expect(mockPost).toHaveBeenCalledWith("/assistant/actions/org/notifications/telegram-disconnect", { actionRequestId: "request-telegram_disconnect" });
  });
});
