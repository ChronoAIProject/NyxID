import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { NotificationSettingsPage } from "./notification-settings";
const mocks = vi.hoisted(() => ({ save: vi.fn().mockResolvedValue({}) }));
vi.mock("@/hooks/use-user-services", () => ({ useUserServices: () => ({ data: [] }) }));
vi.mock("@/components/shared/approval-setup-wizard", () => ({ ApprovalSetupWizard: () => null }));
vi.mock("@/hooks/use-approvals", () => ({
  useNotificationSettings: () => ({ data: { telegram_connected: false, telegram_enabled: false, push_enabled: true, approval_required: true, global_approval_mode: "writes_only", approval_timeout_secs: 300, grant_expiry_days: 30 }, isLoading: false }),
  useUpdateNotificationSettings: () => ({ mutateAsync: mocks.save }),
  useTelegramLink: () => ({}), useTelegramDisconnect: () => ({}),
  usePushDevices: () => ({ data: { devices: [] } }), useRemoveDevice: () => ({}),
  useServiceApprovalConfigs: () => ({ data: { configs: [] } }),
  useSetServiceApprovalConfig: () => ({}), useDeleteServiceApprovalConfig: () => ({}),
}));
it("defaults Mentions in groups on for older settings and saves an explicit opt-out", async () => {
  const user = userEvent.setup();
  render(<NotificationSettingsPage />);
  const toggle = screen.getByRole("switch", { name: "Mentions in groups" });
  expect(toggle).toBeChecked();
  await user.click(toggle);
  expect(toggle).not.toBeChecked();
  await user.click(screen.getByRole("button", { name: "Save Preferences" }));
  await waitFor(() => expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ group_mentions_enabled: false })));
});

it("preserves an omitted mention preference when saving another setting", async () => {
  mocks.save.mockClear();
  const user = userEvent.setup();
  render(<NotificationSettingsPage />);
  const timeout = screen.getByRole("spinbutton", { name: "Approval Timeout (seconds)" });
  await user.clear(timeout);
  await user.type(timeout, "60");
  await user.click(screen.getByRole("button", { name: "Save Preferences" }));
  await waitFor(() => expect(mocks.save).toHaveBeenCalledOnce());
  const saved = mocks.save.mock.lastCall?.[0];
  expect(saved).toEqual(expect.objectContaining({ approval_timeout_secs: 60 }));
  expect(saved).not.toHaveProperty("group_mentions_enabled");
});
