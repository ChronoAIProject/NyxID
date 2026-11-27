import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { NotificationSettings, ServiceApprovalConfigItem } from "@/types/approvals";
const fixture = vi.hoisted(() => ({
  configs: [] as ServiceApprovalConfigItem[],
  save: vi.fn(),
  updateSettings: vi.fn(),
  ready: true,
}));
vi.mock("@/hooks/use-approvals", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/hooks/use-approvals")>(),
  useApprovalAuthoringCapabilities: () => ({
    data: { personal_rules: fixture.ready },
  }),
  useServiceApprovalConfigs: () => ({ data: { configs: fixture.configs } }),
  useSetServiceApprovalConfig: () => ({
    mutateAsync: fixture.save,
    isPending: false,
  }),
  useDeleteServiceApprovalConfig: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useUpdateNotificationSettings: () => ({
    mutateAsync: fixture.updateSettings,
    isPending: false,
  }),
  useApprovalGrants: () => ({ data: { grants: [], total: 0 } }),
  useRevokeGrant: () => ({ mutate: vi.fn() }),
}));
vi.mock("@/hooks/use-user-services", () => ({
  useUserServices: () => ({
    data: [
      {
        id: "connection",
        slug: "weather",
        label: "Weather",
        catalog_service_id: "catalog",
        is_active: true,
        credential_source: { type: "personal" },
      },
    ],
  }),
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useApiKeys: () => ({
    data: [
      { id: "agent-a", name: "Agent A", is_active: true, key_prefix: "nyx_a" },
    ],
  }),
}));
vi.mock("@/hooks/use-endpoints", () => ({
  useEndpoints: () => ({ data: [], isLoading: false, isError: false }),
}));
import { ApprovalRules } from "./approval-rules";
const general = {
  methods: [],
  resource_pattern: "*",
  verbs: ["write", "destructive"],
  effect: "require_approval",
  mode: "grant",
} as const;
const config: ServiceApprovalConfigItem = {
  service_id: "catalog",
  service_name: "Weather",
  user_service_id: "connection",
  approval_required: true,
  approval_mode: "grant",
  rules: [general],
  default_effect: "auto_allow",
  grant_expiry_days: null,
  revision: "revision-1",
  created_at: "old",
  updated_at: "old",
};
const initialSettings: NotificationSettings = {
  global_approval_mode: "off",
  approval_required: false,
  approval_suspended: false,
  push_enabled: false,
  telegram_enabled: false,
  telegram_connected: false,
  telegram_username: null,
  push_device_count: 0,
  approval_timeout_secs: 300,
  grant_expiry_days: 7,
};

function renderRules() {
  const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
  client.setQueryData(["notifications", "settings"], initialSettings);
  return render(<ApprovalRules />, {
    wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  });
}

afterEach(() => vi.restoreAllMocks());

beforeEach(() => {
  fixture.configs = [config];
  fixture.ready = true;
  fixture.updateSettings.mockReset();
  vi.spyOn(api, "get").mockResolvedValue(initialSettings);
  fixture.save.mockReset().mockResolvedValue(config);
});
it("adds a guided key protection while preserving the exact All keys grant rule and null days", async () => {
  const user = userEvent.setup();
  renderRules();
  await user.click(screen.getByRole("button", { name: "Set up a protection" }));
  await user.click(screen.getByRole("radio", { name: /Weather/ }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("radio", { name: /Specific agent keys/ }));
  await user.click(screen.getByRole("checkbox", { name: /Agent A/ }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("button", { name: "Turn on" }));
  await user.click(screen.getAllByRole("button", { name: "Turn on" }).at(-1)!);
  expect(fixture.save).toHaveBeenCalledWith(
    expect.objectContaining({
      serviceId: "connection",
      expectedRevision: "revision-1",
      approvalMode: "grant",
      defaultEffect: "auto_allow",
    }),
  );
  const saved = fixture.save.mock.calls[0]![0];
  expect(saved.rules.at(-1)).toEqual(general);
  expect(
    saved.rules
      .slice(0, -1)
      .every(
        (rule: { api_key_ids: string[] }) =>
          rule.api_key_ids.join() === "agent-a",
      ),
  ).toBe(true);
  expect(saved).not.toHaveProperty("grantExpiryDays");
});
it("routes an unmatched Deny policy to exact editing without a compiler save", async () => {
  fixture.configs = [
    {
      ...config,
      default_effect: "deny",
      rules: [{ ...general, methods: ["EXEC"], resource_pattern: "git *" }],
    },
  ];
  const user = userEvent.setup();
  renderRules();
  await user.click(screen.getByRole("button", { name: "Set up a protection" }));
  await user.click(screen.getByRole("radio", { name: /Weather/ }));
  await user.click(screen.getByRole("button", { name: "Edit stored policy" }));
  expect(
    within(screen.getByRole("dialog")).getByRole("heading", {
      name: "Weather",
    }),
  ).toBeInTheDocument();
  expect(fixture.save).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Save Rules" }));
  expect(fixture.save).toHaveBeenCalledWith(
    expect.objectContaining({
      serviceId: "catalog",
      defaultEffect: "deny",
      rules: fixture.configs[0]!.rules,
      grantExpiryDays: null,
      expectedRevision: "revision-1",
    }),
  );
});
it("keeps basic protections available while the fleet cannot confirm key-limited support", () => {
  fixture.ready = false;
  renderRules();
  expect(
    screen.getByRole("button", { name: "Set up a protection" }),
  ).toBeEnabled();
});

it("pauses a guided key protection when readiness drops before confirmation", async () => {
  const user = userEvent.setup();
  const { rerender } = renderRules();
  await user.click(screen.getByRole("button", { name: "Set up a protection" }));
  await user.click(screen.getByRole("radio", { name: /Weather/ }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("radio", { name: /Specific agent keys/ }));
  await user.click(screen.getByRole("checkbox", { name: /Agent A/ }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  await user.click(screen.getByRole("button", { name: "Turn on" }));
  fixture.ready = false;
  rerender(<ApprovalRules />);
  expect(screen.getAllByRole("button", { name: "Turn on" }).at(-1)).toBeDisabled();
  expect(fixture.save).not.toHaveBeenCalled();
});

it("opens the App Store separately and links Telegram inside the protection draft", async () => {
  fixture.configs = [];
  const user = userEvent.setup();
  const post = vi.spyOn(api, "post").mockResolvedValue({
    bot_username: "PreviewBot", link_code: "NYXID-EXAMPLE1", expires_in_secs: 300,
  });
  renderRules();
  await user.click(screen.getByRole("button", { name: "Set up a protection" }));
  const dialog = within(screen.getByRole("dialog"));
  await user.click(dialog.getByRole("radio", { name: /Weather/ }));
  await user.click(dialog.getByRole("button", { name: "Continue" }));
  await user.click(dialog.getByRole("button", { name: "Continue" }));
  await user.click(dialog.getByRole("button", { name: "Continue" }));
  const store = dialog.getByRole("link", { name: "Get the app" });
  expect(store).toHaveAttribute("href", "https://apps.apple.com/app/nyxid/id6759768423");
  expect(store).toHaveAttribute("target", "_blank");
  await user.click(dialog.getByRole("button", { name: "Connect" }));
  expect(post).toHaveBeenCalledExactlyOnceWith("/notifications/telegram/link");
  const telegram = await dialog.findByRole("link", { name: "Open Telegram" });
  expect(telegram).toHaveAttribute("href", "https://t.me/PreviewBot?start=NYXID-EXAMPLE1");
  expect(telegram).toHaveAttribute("target", "_blank");
  expect(dialog.getByText("/start NYXID-EXAMPLE1")).toBeVisible();
  expect(screen.getAllByRole("dialog")).toHaveLength(1);
  expect(dialog.getByRole("button", { name: "Continue" })).toBeEnabled();

  // The bot confirms outside the browser. Only the settings read reveals completion.
  vi.mocked(api.get).mockResolvedValue({ ...initialSettings, telegram_connected: true, telegram_enabled: true });
  expect(await dialog.findByRole("switch", { name: "Telegram" }, { timeout: 6500 })).toBeChecked();
  expect(dialog.queryByText("/start NYXID-EXAMPLE1")).toBeNull();
  await user.click(dialog.getByRole("button", { name: "Continue" }));
  expect(dialog.getByText("Weather")).toBeVisible();
  expect(dialog.getByText("This Approvals page, Telegram")).toBeVisible();
  await user.click(dialog.getByRole("button", { name: "Turn on" }));
  expect(fixture.save).toHaveBeenCalledTimes(1);
  expect(fixture.updateSettings).not.toHaveBeenCalled();
}, 12000);

it("recovers from generation errors and expired Telegram codes without leaving the channel controls", async () => {
  const user = userEvent.setup();
  const response = { bot_username: "PreviewBot", link_code: "NYXID-OLD", expires_in_secs: 0 };
  const post = vi.spyOn(api, "post")
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce(response)
    .mockResolvedValueOnce({ ...response, link_code: "NYXID-NEW", expires_in_secs: 300 });
  renderRules();
  await user.click(screen.getByRole("button", { name: "Connect" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Failed to generate link code");
  await user.click(screen.getByRole("button", { name: "Connect" }));
  expect(await screen.findByText("This Telegram code has expired.")).toBeVisible();
  expect(screen.queryByRole("link", { name: "Open Telegram" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "Get a new code" }));
  expect(await screen.findByText("/start NYXID-NEW")).toBeVisible();
  expect(post).toHaveBeenCalledTimes(3);
});
