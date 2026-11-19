import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n/instance";
import zh from "@/i18n/locales/app/zh-CN";
import zhTW from "@/i18n/locales/app/zh-TW";
import {
  ComposerConnectionPicker,
  ComposerConnectionChip,
} from "./composer-connection-picker";
import type { KeyInfo } from "@/types/keys";

const key = {
  id: "work",
  label: "Work",
  slug: "work",
  is_active: true,
  status: "active",
  credential_missing: true,
} as KeyInfo;
const props = {
  open: true,
  onOpenChange: vi.fn(),
  connections: [key],
  value: "",
  onChange: vi.fn(),
  loading: false,
  error: false,
  onRetry: vi.fn(),
  disabled: false,
};
afterEach(async () => {
  vi.clearAllMocks();
  await act(() => appI18n.changeLanguage("en"));
});

it("lets keyboard users discover reasons without selecting or clearing search", async () => {
  const user = userEvent.setup();
  render(<ComposerConnectionPicker {...props} />);
  const search = screen.getByRole("textbox");
  await user.type(search, "Work");
  await user.tab();
  const option = screen.getByRole("button", { name: "Work Personal · work" });
  expect(option).toHaveFocus();
  expect(option).toHaveAttribute("aria-disabled", "true");
  expect(option).toBeEnabled();
  expect(option).toHaveAccessibleDescription("Credential missing");
  await user.keyboard("{Enter} ");
  await user.click(option);
  expect(props.onChange).not.toHaveBeenCalled();
  expect(props.onOpenChange).not.toHaveBeenCalled();
  expect(search).toHaveValue("Work");
});

it("uses native disabled only while the entire picker is locked", () => {
  render(<ComposerConnectionPicker {...props} disabled />);
  expect(
    screen.getByRole("button", { name: "Work Personal · work" }),
  ).toBeDisabled();
});

it("uses the wall clock for expiry when no controlled time is provided", () => {
  render(<ComposerConnectionPicker {...props} connections={[{
    ...key, credential_missing: false, credential_type: "api_key",
    expires_at: new Date(Date.now() - 1_000).toISOString(),
  }]} />);
  expect(screen.getByRole("button", { name: "Work Personal · work" }))
    .toHaveAccessibleDescription("Credential expired");
});

it("shows a visible accessible reason when the backend supplies only whitespace", () => {
  render(<ComposerConnectionPicker {...props} connections={[{
    ...key,
    credential_missing: false,
    execution_unavailable_reason: "   ",
  }]} />);
  const option = screen.getByRole("button", { name: "Work Personal · work" });
  expect(option).toHaveAttribute("aria-disabled", "true");
  expect(option).toHaveAccessibleDescription("Connection unavailable");
  expect(screen.getByText("Connection unavailable")).toBeVisible();
});

it.each([
  ["en", "Checking availability…"],
  ["zh-CN", "正在检查可用性…"],
  ["zh-TW", "正在檢查可用性…"],
] as const)("exposes the held option's checking state in %s", async (language, description) => {
  appI18n.addResourceBundle("zh-CN", "app", zh, true, true);
  appI18n.addResourceBundle("zh-TW", "app", zhTW, true, true);
  await act(() => appI18n.changeLanguage(language));
  render(<ComposerConnectionPicker {...props} connections={[{ ...key, credential_missing: false }]}
    checkingId="work" />);
  const option = screen.getByRole("button", { name: /Work/ });
  expect(option).toHaveAttribute("aria-busy", "true");
  expect(option).toHaveAccessibleDescription(description);
  expect(screen.getByText(description)).toBeVisible();
  expect(screen.getByText(description)).toHaveClass("text-muted-foreground");
  expect(screen.getByText(description)).not.toHaveClass("text-destructive");
});

it.each([null, "Credential missing"])("R6-1: checking stays neutral and quiet until a refused send, then settles to %s", (reason) => {
  const chipProps = {
    connection: { ...key, credential_missing: false },
    unavailableReason: null,
    disabled: false,
    onChange: vi.fn(),
    onRemove: vi.fn(),
  };
  const view = render(<ComposerConnectionChip {...chipProps} />);
  const status = screen.getByRole("status");
  const previousText = status.textContent;
  view.rerender(<ComposerConnectionChip {...chipProps} checking />);
  const checking = screen.getByText(/Checking availability/);
  expect(checking).toBeVisible();
  expect(checking).toHaveClass("text-muted-foreground");
  expect(checking).not.toHaveClass("text-destructive");
  expect(checking).toHaveAttribute("aria-hidden", "true");
  expect(status.textContent).toBe(previousText);
  expect(status).not.toHaveTextContent("Checking");
  expect(screen.getByRole("group")).not.toHaveTextContent("Unavailable");
  expect(screen.getByRole("button", { name: "Change connection for this message" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Remove connection for this message" })).toBeEnabled();
  view.rerender(<ComposerConnectionChip {...chipProps} checking announceChecking />);
  expect(status).toHaveTextContent("Checking availability…");
  expect(status).toHaveClass("text-muted-foreground");
  expect(status).not.toHaveClass("text-destructive");
  expect(screen.getByRole("group")).not.toHaveTextContent("Unavailable");
  view.rerender(<ComposerConnectionChip {...chipProps} unavailableReason={reason} />);
  expect(status).not.toHaveTextContent("Checking");
  if (reason) {
    expect(status).toHaveTextContent("Unavailable — choose another connection or remove · Credential missing");
    expect(status).toHaveClass("text-destructive");
  } else {
    expect(status).toHaveTextContent("This message");
    expect(status).toHaveClass("text-muted-foreground");
  }
});

it("R6-1: preserves the previous unavailable announcement while a background refresh is pending", () => {
  const chipProps = { connection: key, disabled: false, onChange: vi.fn(), onRemove: vi.fn() };
  const view = render(<ComposerConnectionChip {...chipProps} unavailableReason="Credential missing" />);
  const status = screen.getByRole("status");
  const previousText = status.textContent;
  view.rerender(<ComposerConnectionChip {...chipProps} unavailableReason={null} checking />);
  expect(status.textContent).toBe(previousText);
  expect(status).toHaveClass("sr-only");
  expect(status).not.toHaveTextContent("Checking");
  expect(screen.getByText(/Checking availability/)).toHaveClass("text-muted-foreground");
  view.rerender(<ComposerConnectionChip {...chipProps} unavailableReason={null} />);
  expect(status).toHaveTextContent("This message");
  expect(status).not.toHaveClass("sr-only");
});

it("localizes node and personal copy with literal names, and preserves unknown backend text", async () => {
  appI18n.addResourceBundle("zh-CN", "app", zh, true, true);
  await act(() => appI18n.changeLanguage("zh-CN"));
  const name = "Home {{x}} <b>";
  const node = {
    ...key,
    credential_missing: false,
    execution_unavailable_reason: `node ${name} offline`,
  };
  const unknown = {
    ...node,
    id: "other",
    label: "Other",
    execution_unavailable_reason: "Provider {{x}} <b>wait</b>",
  };
  render(<ComposerConnectionPicker {...props} connections={[node, unknown]} />);
  expect(
    screen.getByRole("button", { name: "Work 个人 · work" }),
  ).toHaveAccessibleDescription(`节点 ${name} 已离线`);
  expect(
    screen.getByRole("button", { name: "Other 个人 · work" }),
  ).toHaveAccessibleDescription(unknown.execution_unavailable_reason);
  expect(document.querySelector("b")).toBeNull();
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "个人" } });
  expect(
    screen.getByRole("button", { name: "Work 个人 · work" }),
  ).toBeVisible();
});

it("keeps organization names literal in the picker, chip and search", async () => {
  appI18n.addResourceBundle("zh-CN", "app", zh, true, true);
  await act(() => appI18n.changeLanguage("zh-CN"));
  const org = {
    ...key,
    credential_source: {
      type: "org",
      org_id: "org",
      org_name: "Personal",
      role: "member",
      allowed: true,
    },
  } as KeyInfo;
  render(
    <>
      <ComposerConnectionPicker {...props} connections={[org]} />
      <ComposerConnectionChip
        connection={org}
        unavailableReason={null}
        disabled={false}
        onRemove={vi.fn()}
        onChange={vi.fn()}
      />
    </>,
  );
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Personal" },
  });
  expect(
    screen.getByRole("button", { name: "Work Personal · work" }),
  ).toBeVisible();
  expect(
    screen.getByRole("group", { name: "此消息使用的连接", hidden: true }),
  ).toHaveTextContent("Personal");
});

it("shows a visible Change action whose accessible name contains the label", async () => {
  const onChange = vi.fn();
  render(<ComposerConnectionChip connection={key} unavailableReason="Credential missing" disabled={false} onChange={onChange} onRemove={vi.fn()} />);
  const change = screen.getByRole("button", { name: "Change connection for this message" });
  expect(change).toHaveTextContent("Change");
  await userEvent.setup().click(change);
  expect(onChange).toHaveBeenCalledOnce();
});

it.each([
  ["en", ["Node home is offline", "Node home is draining", "Node home is disabled", "Node home is unavailable", "Node home is unavailable"]],
  ["zh-CN", ["节点 home 已离线", "节点 home 正在排空", "节点 home 已停用", "节点 home 不可用", "节点 home 不可用"]],
  ["zh-TW", ["節點 home 已離線", "節點 home 正在排空", "節點 home 已停用", "節點 home 無法使用", "節點 home 無法使用"]],
] as const)("localizes known node states and hides raw fallback status in %s", async (language, descriptions) => {
  appI18n.addResourceBundle("zh-CN", "app", zh, true, true);
  appI18n.addResourceBundle("zh-TW", "app", zhTW, true, true);
  await act(() => appI18n.changeLanguage(language));
  const connections = ["offline", "draining", "disabled", "online", "unknown"].map((status, index) => ({
    ...key, id: `node-${index}`, label: `Node ${index}`, credential_missing: false,
    execution_unavailable_reason: `node home ${status}`,
  }));
  render(<ComposerConnectionPicker {...props} connections={connections} />);
  connections.forEach((connection, index) => {
    expect(screen.getByRole("button", { name: new RegExp(connection.label) })).toHaveAccessibleDescription(descriptions[index]);
  });
});
