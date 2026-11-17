import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n/instance";
import zh from "@/i18n/locales/app/zh-CN";
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
