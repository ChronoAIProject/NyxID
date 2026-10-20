import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import type { NotificationSettings } from "@/types/approvals";

const state = vi.hoisted(() => ({ settings: {} as NotificationSettings, update: vi.fn() }));
vi.mock("@tanstack/react-router", () => ({ Link: ({ children, to }: PropsWithChildren<{ to: string }>) => <a href={to}>{children}</a> }));
vi.mock("@/hooks/use-approvals", () => ({
  useNotificationSettings: () => ({ data: state.settings, isLoading: false }),
  usePushDevices: () => ({ data: { devices: [] } }),
  useUpdateNotificationSettings: () => ({ mutateAsync: state.update, isPending: false }),
  useTelegramLink: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
import { ApprovalSetupWizard } from "./approval-setup-wizard";
import { NotificationSetupCard } from "../dashboard/notification-setup-card";

beforeEach(() => {
  vi.clearAllMocks(); localStorage.clear();
  state.settings = { telegram_connected: false, telegram_username: null, telegram_enabled: false, push_enabled: false, push_device_count: 0, approval_required: true, approval_suspended: false, approval_timeout_secs: 300, grant_expiry_days: 30, approval_web_available: true, approval_delivery_configured: false };
});

it("keeps required approval active with web decisions and no notifications", () => {
  render(<NotificationSetupCard />);
  expect(screen.getByText("Approval protection is active")).toBeInTheDocument();
  expect(screen.getByText("Review approvals on the web")).toBeInTheDocument();
  expect(screen.queryByText(/suspended/i)).not.toBeInTheDocument();
});

it("describes the writes-only default without claiming reads require approval", () => {
  state.settings = { ...state.settings, global_approval_mode: "writes_only" };
  const card = render(<NotificationSetupCard />);
  expect(screen.getByText(/Writes and destructive operations require approval; reads and stateless model inference can proceed/)).toBeInTheDocument();
  card.unmount();
  render(<ApprovalSetupWizard hasChannel={false} channelEnabled={false} approvalEnabled approvalMode="writes_only" approvalSuspended={false} webAvailable />);
  expect(screen.getByText(/Writes and destructive operations require approval by default; reads and stateless model inference can proceed/)).toBeInTheDocument();
});

it("allows explicit web-only activation without a registered channel", async () => {
  state.settings = { ...state.settings, approval_required: false };
  render(<NotificationSetupCard />);
  await userEvent.click(screen.getByRole("button", { name: "Enable Approval Protection" }));
  expect(state.update).toHaveBeenCalledWith({ approval_required: true });
});

it("preserves the older server's suspension warning", () => {
  state.settings = { ...state.settings, approval_suspended: true, approval_web_available: undefined, approval_delivery_configured: undefined };
  render(<NotificationSetupCard />);
  expect(screen.getByRole("status")).toHaveTextContent("Server reports approval is suspended");
  expect(screen.queryByText("Approval protection is active")).not.toBeInTheDocument();
  expect(screen.getByText("Notification provider configuration is unknown.")).toBeInTheDocument();
});

it("distinguishes unknown web/provider capability from explicit unavailability", () => {
  state.settings = { ...state.settings, approval_web_available: undefined, approval_delivery_configured: undefined };
  render(<NotificationSetupCard />);
  expect(screen.getByRole("status")).toHaveTextContent("Web-only approval support is unconfirmed");
  expect(screen.queryByText("Review approvals on the web")).not.toBeInTheDocument();
});

it("offers Approval History when server confirms web-only protection", () => {
  render(<ApprovalSetupWizard hasChannel={false} channelEnabled={false} approvalEnabled approvalSuspended={false} webAvailable deliveryConfigured={false} />);
  expect(screen.getByText("Approval protection is active")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Approval History" })).toHaveAttribute("href", "/approvals/history");
  expect(screen.getByText(/Notifications are unavailable; use the web to decide/)).toBeInTheDocument();
});

it("does not claim web-only enforcement for an older unknown server", () => {
  render(<ApprovalSetupWizard hasChannel channelEnabled approvalEnabled approvalSuspended={false} />);
  expect(screen.getByText("Web-only approval support is unconfirmed")).toBeInTheDocument();
  expect(screen.queryByText("Approval protection is active")).not.toBeInTheDocument();
});

it.each([undefined, false])("requires a channel for inactive servers without explicit web support (%s)", (webAvailable) => {
  state.settings = { ...state.settings, approval_required: false, approval_web_available: webAvailable };
  const card = render(<NotificationSetupCard />);
  expect(screen.getByRole("button", { name: "Enable Approval Protection" })).toBeDisabled();
  expect(screen.getByText(/Connect and enable a notification channel before turning on approval protection/)).toBeInTheDocument();
  expect(screen.queryByText(/optional notifications/)).not.toBeInTheDocument();
  card.unmount();
  render(<ApprovalSetupWizard hasChannel={false} channelEnabled={false} approvalEnabled={false} approvalSuspended={false} webAvailable={webAvailable} />);
  expect(screen.getByText(/Connect and enable a notification channel before turning on approval protection/)).toBeInTheDocument();
  expect(screen.queryByText(/Approval protection can be enabled without notifications/)).not.toBeInTheDocument();
  expect(screen.queryByText(/optional notification channel/)).not.toBeInTheDocument();
});

it("explains modern web-only activation while inactive", () => {
  render(<ApprovalSetupWizard hasChannel={false} channelEnabled={false} approvalEnabled={false} approvalSuspended={false} webAvailable />);
  expect(screen.getByText(/Approval protection can be enabled without notifications/)).toBeInTheDocument();
  expect(screen.getByText(/Connect an optional notification channel/)).toBeInTheDocument();
});
