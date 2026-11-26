import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { ServiceApprovalConfigItem } from "@/types/approvals";
const state = vi.hoisted(() => ({
  save: vi.fn(),
  remove: vi.fn(),
  ready: true,
  error: null as Error | null,
}));
vi.mock("@/hooks/use-approvals", () => ({
  useSetServiceApprovalConfig: () => ({
    mutateAsync: state.save,
    isPending: false,
    error: state.error,
  }),
  useDeleteServiceApprovalConfig: () => ({
    mutateAsync: state.remove,
    isPending: false,
    error: null,
  }),
  useApprovalAuthoringCapabilities: () => ({
    data: { personal_rules: state.ready },
  }),
}));
import { StoredApprovalPolicyDialog } from "./stored-approval-policy-dialog";
const config: ServiceApprovalConfigItem = {
  service_id: "legacy",
  service_name: "SSH and HTTP",
  approval_required: true,
  approval_mode: "grant",
  default_effect: "deny",
  grant_expiry_days: null,
  revision: "revision-1",
  created_at: "2026-01-01",
  updated_at: "2026-01-01",
  rules: [
    {
      methods: ["EXEC"],
      resource_pattern: "git *",
      verbs: ["write"],
      effect: "require_approval",
      mode: "grant",
    },
    {
      methods: ["GET"],
      resource_pattern: "/status",
      verbs: [],
      effect: "deny",
      mode: "per_request",
    },
    {
      methods: ["GET"],
      resource_pattern: "/**",
      verbs: [],
      effect: "auto_allow",
      mode: "per_request",
    },
  ],
};
beforeEach(() => {
  state.save.mockReset().mockResolvedValue(config);
  state.remove.mockReset().mockResolvedValue({});
  state.ready = true;
  state.error = null;
});
it("saves a legacy policy in the exact original order with null days and unmatched deny", async () => {
  const close = vi.fn();
  render(
    <StoredApprovalPolicyDialog config={config} apiKeys={[]} onClose={close} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Save Rules" }));
  expect(state.save).toHaveBeenCalledWith({
    serviceId: "legacy",
    approvalRequired: true,
    approvalMode: "grant",
    rules: config.rules,
    defaultEffect: "deny",
    grantExpiryDays: null,
    disabled: false,
    expectedRevision: "revision-1",
  });
  expect(close).toHaveBeenCalled();
});
it("disables without recompiling or removing any stored rule", async () => {
  render(
    <StoredApprovalPolicyDialog
      config={config}
      apiKeys={[]}
      onClose={vi.fn()}
    />,
  );
  await userEvent.click(screen.getByRole("switch", { name: "Enabled" }));
  await userEvent.click(screen.getByRole("button", { name: "Save Rules" }));
  expect(state.save).toHaveBeenCalledWith(
    expect.objectContaining({
      disabled: true,
      rules: config.rules,
      defaultEffect: "deny",
    }),
  );
});
it("removes an empty stale key rule instead of widening its allow to all callers", async () => {
  const scoped = {
    ...config,
    rules: [{ ...config.rules[2]!, api_key_ids: ["deleted"] }],
  };
  render(
    <StoredApprovalPolicyDialog
      config={scoped}
      apiKeys={[]}
      onClose={vi.fn()}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Remove deleted key" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Save Rules" }));
  expect(state.save).toHaveBeenCalledWith(
    expect.objectContaining({ rules: [] }),
  );
});
it("reorders only after an explicit move and keeps all rule fields", async () => {
  render(
    <StoredApprovalPolicyDialog
      config={config}
      apiKeys={[]}
      onClose={vi.fn()}
    />,
  );
  await userEvent.click(
    screen.getAllByRole("button", { name: "Move rule up" })[1]!,
  );
  await userEvent.click(screen.getByRole("button", { name: "Save Rules" }));
  expect(state.save).toHaveBeenCalledWith(
    expect.objectContaining({
      rules: [config.rules[1], config.rules[0], config.rules[2]],
    }),
  );
});
it("pins confirmed removal to the reviewed revision", async () => {
  render(
    <StoredApprovalPolicyDialog
      config={config}
      apiKeys={[]}
      onClose={vi.fn()}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Remove protection" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Confirm removal" }),
  );
  expect(state.remove).toHaveBeenCalledWith({
    serviceId: "legacy",
    expectedRevision: "revision-1",
  });
});
it("preserves the draft and shows a conflict instead of silently overwriting", async () => {
  state.error = new Error("Approval policy changed; reload before saving");
  state.save.mockRejectedValue(state.error);
  const close = vi.fn();
  render(
    <StoredApprovalPolicyDialog config={config} apiKeys={[]} onClose={close} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Save Rules" }));
  expect(screen.getByText(state.error.message)).toBeInTheDocument();
  expect(close).not.toHaveBeenCalled();
  expect(
    within(screen.getByRole("dialog")).getByDisplayValue("git *"),
  ).toBeInTheDocument();
});
it("keeps ordinary edits and removal but waits on new controls until the fleet reports support", async () => {
  state.ready = false;
  render(
    <StoredApprovalPolicyDialog
      config={config}
      apiKeys={[]}
      onClose={vi.fn()}
    />,
  );
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByRole("switch", { name: "Enabled" })).toBeDisabled();
  expect(
    within(dialog).getByLabelText("Remembered approval days"),
  ).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Remove protection" }),
  ).toBeEnabled();
  const saveRules = screen.getByRole("button", { name: "Save Rules" });
  expect(saveRules).toBeEnabled();
  await userEvent.click(saveRules);
  expect(state.save).toHaveBeenCalledOnce();
  const saved = state.save.mock.calls[0]![0];
  expect(saved).not.toHaveProperty("grantExpiryDays");
  expect(saved).not.toHaveProperty("disabled");
  expect(saved).toMatchObject({
    serviceId: "legacy",
    expectedRevision: "revision-1",
  });
});

const apiKeys = [{ id: "agent-a", name: "Agent A" }];

it("keeps key selection unavailable while ordinary unkeyed edits can be saved", async () => {
  state.ready = false;
  const user = userEvent.setup();
  render(
    <StoredApprovalPolicyDialog config={config} apiKeys={apiKeys} onClose={vi.fn()} />,
  );
  for (const checkbox of screen.getAllByRole("checkbox", { name: "Agent A" })) {
    expect(checkbox).toBeDisabled();
  }
  await user.click(screen.getByRole("button", { name: "Save Rules" }));
  expect(state.save).toHaveBeenCalledWith(
    expect.objectContaining({ rules: config.rules }),
  );
});

it("keeps a stored keyed policy read-only until the fleet reports support", async () => {
  state.ready = false;
  const scoped = {
    ...config,
    rules: [{ ...config.rules[2]!, api_key_ids: ["agent-a", "deleted"] }],
  };
  render(
    <StoredApprovalPolicyDialog config={scoped} apiKeys={apiKeys} onClose={vi.fn()} />,
  );
  expect(screen.getByRole("checkbox", { name: "Agent A" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Agent A" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Remove deleted key" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Save Rules" })).toBeDisabled();
  expect(state.save).not.toHaveBeenCalled();
});

it("preserves a scoped draft and disables save if readiness drops", async () => {
  const user = userEvent.setup();
  const close = vi.fn();
  const dialog = () => (
    <StoredApprovalPolicyDialog config={config} apiKeys={apiKeys} onClose={close} />
  );
  const { rerender } = render(dialog());
  await user.click(screen.getAllByRole("checkbox", { name: "Agent A" })[2]!);
  state.ready = false;
  rerender(dialog());
  expect(screen.getAllByRole("checkbox", { name: "Agent A" })[2]).toBeChecked();
  expect(screen.getByRole("button", { name: "Save Rules" })).toBeDisabled();
  expect(state.save).not.toHaveBeenCalled();
  expect(close).not.toHaveBeenCalled();
  state.ready = true;
  rerender(dialog());
  expect(screen.getByRole("button", { name: "Save Rules" })).toBeEnabled();
});
