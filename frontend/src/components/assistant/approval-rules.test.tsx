import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { ServiceApprovalConfigItem } from "@/types/approvals";
const fixture = vi.hoisted(() => ({
  configs: [] as ServiceApprovalConfigItem[],
  save: vi.fn(),
  ready: true,
}));
vi.mock("@/hooks/use-approvals", () => ({
  useApprovalAuthoringCapabilities: () => ({
    data: { personal_rules: fixture.ready },
  }),
  useNotificationSettings: () => ({
    data: {
      global_approval_mode: "off",
      approval_required: false,
      push_enabled: false,
      telegram_enabled: false,
      telegram_linked: false,
      push_devices: [],
      approval_timeout_secs: 300,
      grant_expiry_days: 7,
    },
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
    mutateAsync: vi.fn(),
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
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
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
beforeEach(() => {
  fixture.configs = [config];
  fixture.ready = true;
  fixture.save.mockReset().mockResolvedValue(config);
});
it("adds a guided key protection while preserving the exact All keys grant rule and null days", async () => {
  const user = userEvent.setup();
  render(<ApprovalRules />);
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
  render(<ApprovalRules />);
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
  render(<ApprovalRules />);
  expect(
    screen.getByRole("button", { name: "Set up a protection" }),
  ).toBeEnabled();
});

it("pauses a guided key protection when readiness drops before confirmation", async () => {
  const user = userEvent.setup();
  const { rerender } = render(<ApprovalRules />);
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
