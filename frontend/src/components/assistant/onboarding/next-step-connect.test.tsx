import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NextStepCards } from "./next-step-cards";
import { appI18n } from "@/i18n";

const mocks = vi.hoisted(() => ({ prepare: vi.fn(), connect: vi.fn() }));
vi.mock("@/lib/assistant/nyxbot-api", () => ({ nyxBotApi: { suggestionSetup: mocks.prepare } }));
vi.mock("@/hooks/use-connect-links", () => ({ createAssistantConnectLink: mocks.connect }));
vi.mock("@/components/assistant/assistant-link-modals", () => ({
  AssistantLinkModal: ({ href, onClose }: { href: string | null; onClose: () => void }) => href ? <div role="dialog"><a href={href}>Hosted connection</a><button onClick={onClose}>Close connection</button></div> : null,
}));
const step = { id: "connect_calendar", title: "Connect your calendar", description: "Plan your day", icon: "calendar", prompt: "Connect calendar", connect: true };
const choices = [
  { slug: "api-google-calendar", name: "Google Calendar", scopes: [] },
  { slug: "api-microsoft", name: "Microsoft 365", scopes: ["Calendars.ReadWrite"] },
];
const ready = { connected: true, choices: [], prompt: "Use my existing connected account. Help me plan my day." };
beforeEach(() => {
  vi.resetAllMocks();
  mocks.prepare.mockResolvedValue({ connected: false, choices, prompt: ready.prompt });
  mocks.connect.mockResolvedValue("https://nyxid.test/connect/nyx_clk_token");
});
afterEach(async () => { cleanup(); await appI18n.changeLanguage("en"); });

it.each(choices)("opens real hosted setup for $name, then continues only after the account is connected", async (choice) => {
  const onStart = vi.fn();
  const user = userEvent.setup();
  render(<NextStepCards steps={[step]} onStart={onStart} />);
  expect(mocks.prepare).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  await user.click(await screen.findByRole("button", { name: `Connect ${choice.name}` }));
  await screen.findByRole("dialog");
  expect(mocks.connect).toHaveBeenCalledWith({ serviceSlug: choice.slug, personal: false, scopes: choice.scopes });
  expect(screen.getByRole("link", { name: "Hosted connection" }).getAttribute("href")).toContain("/connect/nyx_clk_");
  expect(onStart).not.toHaveBeenCalled();
  mocks.prepare.mockResolvedValue(ready);
  await user.click(screen.getByRole("button", { name: "Close connection" }));
  await waitFor(() => expect(onStart).toHaveBeenCalledOnce());
  expect(onStart).toHaveBeenCalledWith({ ...step, connect: false, prompt: ready.prompt });
});

it("omits a provider the server cannot connect", async () => {
  mocks.prepare.mockResolvedValue({ connected: false, choices: choices.slice(0, 1), prompt: ready.prompt });
  render(<NextStepCards steps={[step]} onStart={vi.fn()} />);
  await userEvent.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  await screen.findByRole("button", { name: "Connect Google Calendar" });
  expect(screen.queryByRole("button", { name: "Connect Microsoft 365" })).toBeNull();
});

it.each([false, true])("rechecks stale suggestions and reuses an existing account (connected before choice: %s)", async (beforeChoice) => {
  const onStart = vi.fn();
  if (beforeChoice) mocks.prepare.mockResolvedValue(ready);
  render(<NextStepCards steps={[step]} onStart={onStart} />);
  await userEvent.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  if (!beforeChoice) {
    await screen.findByRole("button", { name: "Connect Google Calendar" });
    mocks.prepare.mockResolvedValue(ready);
    await userEvent.click(screen.getByRole("button", { name: "Connect Google Calendar" }));
  }
  await waitFor(() => expect(onStart).toHaveBeenCalledOnce());
  expect(mocks.connect).not.toHaveBeenCalled();
});

it("keeps legacy and non-connection suggestions on their existing chat path", async () => {
  const onStart = vi.fn();
  const legacy = { ...step, connect: undefined };
  render(<NextStepCards steps={[legacy]} onStart={onStart} />);
  await userEvent.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  expect(onStart).toHaveBeenCalledWith(legacy);
  expect(mocks.prepare).not.toHaveBeenCalled();
});

it.each(["first read", "selection recheck"])("starts the original prompt exactly once without minting when choices disappear on %s", async (when) => {
  const unavailable = { connected: false, choices: [], prompt: ready.prompt };
  if (when === "first read") mocks.prepare.mockResolvedValue(unavailable);
  const onStart = vi.fn();
  render(<NextStepCards steps={[step]} onStart={onStart} />);
  await userEvent.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  if (when === "selection recheck") {
    await screen.findByRole("button", { name: "Connect Google Calendar" });
    mocks.prepare.mockResolvedValue(unavailable);
    await userEvent.click(screen.getByRole("button", { name: "Connect Google Calendar" }));
  }
  await waitFor(() => expect(onStart).toHaveBeenCalledOnce());
  expect(onStart).toHaveBeenCalledWith({ ...step, connect: false });
  expect(mocks.connect).not.toHaveBeenCalled();
  expect(screen.queryByLabelText(`${step.title} options`)).toBeNull();
  expect(screen.queryByText(/No supported provider/)).toBeNull();
});

it("starts the original prompt if no providers remain after hosted setup closes", async () => {
  const onStart = vi.fn();
  render(<NextStepCards steps={[step]} onStart={onStart} />);
  await userEvent.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  await userEvent.click(await screen.findByRole("button", { name: "Connect Google Calendar" }));
  await screen.findByRole("dialog");
  mocks.prepare.mockResolvedValue({ connected: false, choices: [], prompt: ready.prompt });
  await userEvent.click(screen.getByRole("button", { name: "Close connection" }));
  await waitFor(() => expect(onStart).toHaveBeenCalledOnce());
  expect(onStart).toHaveBeenCalledWith({ ...step, connect: false });
  expect(mocks.connect).toHaveBeenCalledOnce();
  expect(screen.queryByLabelText(`${step.title} options`)).toBeNull();
});

it("finishes the selected setup when a Home refresh removes its completed suggestion", async () => {
  const onStart = vi.fn();
  const user = userEvent.setup();
  const view = render(<NextStepCards steps={[step]} onStart={onStart} />);
  await user.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  await user.click(await screen.findByRole("button", { name: "Connect Google Calendar" }));
  await screen.findByRole("dialog");
  mocks.prepare.mockResolvedValue(ready);
  view.rerender(<NextStepCards steps={[]} onStart={onStart} />);
  expect(screen.getByRole("dialog")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Close connection" }));
  await waitFor(() => expect(onStart).toHaveBeenCalledOnce());
  expect(mocks.connect).toHaveBeenCalledOnce();
});

it.each([
  { language: "en", checking: "Checking your connections…", choose: "Choose a provider to connect your account.", connect: "Connect Google Calendar", cancel: "Cancel", retry: "Try again", error: "Could not load connection options. Try again.", options: `${step.title} options` },
  { language: "zh-CN", checking: "正在检查已连接的账户…", choose: "选择服务商以连接你的账户。", connect: "连接 Google Calendar", cancel: "取消", retry: "再试一次", error: "无法加载连接选项，请重试。", options: `${step.title}选项` },
  { language: "zh-TW", checking: "正在檢查已連結的帳號…", choose: "選擇服務供應商以連結你的帳號。", connect: "連結 Google Calendar", cancel: "取消", retry: "再試一次", error: "無法載入連結選項，請重試。", options: `${step.title}選項` },
])("localizes chooser controls, status and errors in $language", async (copy) => {
  await appI18n.changeLanguage(copy.language);
  let resolveSetup!: (value: { connected: boolean; choices: typeof choices; prompt: string }) => void;
  mocks.prepare.mockReturnValueOnce(new Promise((resolve) => { resolveSetup = resolve; }));
  render(<NextStepCards steps={[step]} onStart={vi.fn()} />);
  await userEvent.click(screen.getByRole("button", { name: /Connect your calendar/ }));
  expect(screen.getByText(copy.checking)).toBeTruthy();
  expect(screen.getByLabelText(copy.options)).toBeTruthy();
  await act(async () => resolveSetup({ connected: false, choices, prompt: ready.prompt }));
  expect(await screen.findByText(copy.choose)).toBeTruthy();
  expect(screen.getByRole("button", { name: copy.cancel })).toBeTruthy();
  mocks.prepare.mockRejectedValueOnce(new Error("Unavailable"));
  await userEvent.click(screen.getByRole("button", { name: copy.connect }));
  expect((await screen.findByRole("alert")).textContent).toBe(copy.error);
  expect(screen.getByRole("button", { name: copy.retry })).toBeTruthy();
  expect(mocks.connect).not.toHaveBeenCalled();
  // An already visible error must follow a later UI-language switch too.
  await act(async () => { await appI18n.changeLanguage("en"); });
  expect(screen.getByRole("alert").textContent).toBe("Could not load connection options. Try again.");
});
