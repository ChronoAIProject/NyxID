import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({
  search: vi.fn(),
  setup: vi.fn(),
  create: vi.fn(),
  update: vi.fn(),
  preview: vi.fn(),
  saveSettings: vi.fn(),
  settings: vi.fn(),
  triggers: vi.fn(),
  remove: vi.fn(),
  run: vi.fn(),
  history: vi.fn(),
  older: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => mocks.search(),
  Link: ({
    to,
    search,
    children,
    ...props
  }: {
    to: string;
    search?: Record<string, string>;
    children: React.ReactNode;
  }) => (
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props}>
      {children}
    </a>
  ),
}));
vi.mock("@/hooks/use-triggers", () => ({
  useCreateTrigger: () => ({ mutateAsync: mocks.create }),
  useUpdateTrigger: () => ({ mutateAsync: mocks.update }),
  useDeleteTrigger: () => ({ mutateAsync: mocks.remove }),
  useTriggers: mocks.triggers,
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({
    data: { agents: [{ id: "nyxbot", name: "nyxbot", status: "active" }] },
  }),
  useNyxBotSettings: mocks.settings,
  useUpdateNyxBotSettings: () => ({ mutateAsync: mocks.saveSettings }),
}));
vi.mock("@/hooks/use-automations", () => ({
  useAutomationSetup: mocks.setup,
  useSchedulePreview: mocks.preview,
  useAutomationChats: () => ({
    data: [{ id: "telegram", bot_label: "My bot", title: "Owner chat" }],
  }),
  useRunAutomation: () => ({ mutateAsync: mocks.run }),
  useAutomationRuns: mocks.history,
}));
import { AutomationEditor, AutomationsPage } from "./automations";
import { AutomationPreferences } from "@/components/assistant/automation-preferences";
import type { TriggerResponse } from "@/schemas/triggers";
const trigger: TriggerResponse = {
  id: "80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  user_id: "owner",
  label: "Morning summary",
  source: "schedule",
  status: "active",
  user_service_id: null,
  schedule: {
    kind: "cron",
    expression: "0 8 * * 1-5",
    timezone: "Asia/Singapore",
  },
  verification: { mode: "schedule" },
  delivery: {
    type: "assistant",
    agent_id: "nyxbot",
    instruction: "Summarise GitHub",
    deliver_to: { type: "thread" },
  },
  inbound_url: null,
  created_at: "2026-10-01T00:00:00Z",
  updated_at: "2026-10-01T00:00:00Z",
};
beforeEach(() => {
  vi.clearAllMocks();
  mocks.search.mockReturnValue({});
  mocks.setup.mockReturnValue({});
  window.history.replaceState({}, "", "/automations");
  mocks.settings.mockReturnValue({ data: { timezone: "Asia/Singapore" } });
  mocks.triggers.mockReturnValue({ data: { triggers: [] } });
  mocks.history.mockReturnValue({ data: { pages: [] } });
  mocks.update.mockResolvedValue({ trigger });
  mocks.remove.mockResolvedValue({});
  mocks.run.mockResolvedValue({});
  mocks.preview.mockReturnValue({
    data: {
      description: "At 08:00 Monday–Friday, Asia/Singapore",
      next_runs: ["2026-10-02T08:00:00+08:00"],
    },
  });
  mocks.create.mockResolvedValue({
    trigger: { inbound_url: null },
    secret: null,
    delivery_signing_secret: null,
  });
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function chooseOption(label: string, name: string) {
  fireEvent.keyDown(screen.getByLabelText(label), { key: "ArrowDown" });
  fireEvent.keyDown(screen.getByRole("option", { name }), { key: "Enter" });
}

it("previews calendar times after the debounce and cancels stale previews", async () => {
  vi.useFakeTimers();
  render(<AutomationEditor onClose={vi.fn()} onSecrets={vi.fn()} />);
  expect(mocks.preview).toHaveBeenLastCalledWith(undefined);
  await act(() => vi.advanceTimersByTimeAsync(349));
  expect(mocks.preview).toHaveBeenLastCalledWith(undefined);
  await act(() => vi.advanceTimersByTimeAsync(1));
  expect(mocks.preview).toHaveBeenLastCalledWith(
    expect.objectContaining({ kind: "cron", timezone: "Asia/Singapore" }),
  );
  expect(
    screen.getByText("At 08:00 Monday–Friday, Asia/Singapore"),
  ).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Cron expression"), {
    target: { value: "0 9 * * *" },
  });
  await act(() => vi.advanceTimersByTimeAsync(200));
  fireEvent.change(screen.getByLabelText("Cron expression"), {
    target: { value: "0 10 * * *" },
  });
  await act(() => vi.advanceTimersByTimeAsync(349));
  expect(mocks.preview).not.toHaveBeenLastCalledWith(
    expect.objectContaining({ expression: "0 9 * * *" }),
  );
  await act(() => vi.advanceTimersByTimeAsync(1));
  expect(mocks.preview).toHaveBeenLastCalledWith(
    expect.objectContaining({ expression: "0 10 * * *" }),
  );
});

it("creates an agent schedule with a chat result", async () => {
  const user = userEvent.setup();
  const close = vi.fn();
  render(<AutomationEditor onClose={close} onSecrets={vi.fn()} />);
  fireEvent.change(screen.getByLabelText("Label"), {
    target: { value: "GitHub summary" },
  });
  chooseOption("Target agent", "nyxbot");
  fireEvent.change(screen.getByLabelText("Instruction"), {
    target: { value: "Summarise GitHub notifications" },
  });
  chooseOption("Send result to", "Channel chat");
  chooseOption("Chat (posting must be allowed)", "My bot: Owner chat");
  await user.click(screen.getByRole("button", { name: "Save automation" }));
  await waitFor(() =>
    expect(mocks.create).toHaveBeenCalledWith(
      expect.objectContaining({
        source: "schedule",
        label: "GitHub summary",
        delivery: expect.objectContaining({
          agent_id: "nyxbot",
          instruction: "Summarise GitHub notifications",
          deliver_to: { type: "chat", chat_id: "telegram" },
        }),
      }),
    ),
  );
  expect(close).toHaveBeenCalled();
});
it("prefills a watched webhook and reveals its secret only to the page", async () => {
  mocks.create.mockResolvedValue({
    trigger: { inbound_url: "https://nyx.example/webhook" },
    secret: "nyx_trg_once",
    delivery_signing_secret: null,
  });
  const secrets = vi.fn();
  const user = userEvent.setup();
  render(
    <AutomationEditor
      onClose={vi.fn()}
      onSecrets={secrets}
      watchId="watch-123"
      prefill={{
        label: "Triage",
        instruction: "Review issues",
        agent_id: "nyxbot",
        confirmation_policy: "changes",
        thread_policy: "home",
      }}
    />,
  );
  expect(screen.getByLabelText("Instruction")).toHaveValue("Review issues");
  expect(screen.getByLabelText("Source")).toHaveTextContent("Webhook");
  expect(screen.getByLabelText("Thread")).toHaveTextContent("Home thread");
  expect(screen.getByText(/can influence later owner turns/)).toHaveTextContent(
    "private channel chats",
  );
  await user.click(screen.getByRole("button", { name: "Save automation" }));
  await waitFor(() =>
    expect(mocks.create).toHaveBeenCalledWith(
      expect.objectContaining({
        source: "webhook",
        watch_id: "watch-123",
        delivery: expect.objectContaining({ thread_policy: "home" }),
      }),
    ),
  );
  expect(secrets).toHaveBeenCalledWith([
    { label: "Inbound URL", value: "https://nyx.example/webhook" },
    { label: "Inbound secret", value: "nyx_trg_once" },
  ]);
});
it("shows invalid preview responses without hiding the form", () => {
  mocks.preview.mockReturnValue({
    error: new Error("Cron cadence is below the owner's minimum interval"),
  });
  render(<AutomationEditor onClose={vi.fn()} onSecrets={vi.fn()} />);
  expect(screen.getByRole("alert")).toHaveTextContent("minimum interval");
});
it("adopts a late-loaded owner timezone while preserving an edited timezone", async () => {
  // This case tests preference hydration, not the browser's complete zone inventory.
  vi.spyOn(Intl, "supportedValuesOf").mockReturnValue([
    "Asia/Singapore",
    "America/New_York",
    "America/Chicago",
  ]);
  const user = userEvent.setup();
  mocks.settings.mockReturnValue({ isPending: true });
  const editor = <AutomationEditor onClose={vi.fn()} onSecrets={vi.fn()} />;
  const view = render(editor);
  mocks.settings.mockReturnValue({ data: { timezone: "Asia/Singapore" } });
  view.rerender(<AutomationEditor onClose={vi.fn()} onSecrets={vi.fn()} />);
  expect(screen.getByLabelText("IANA timezone")).toHaveTextContent(
    "Asia/Singapore",
  );
  await user.click(screen.getByLabelText("IANA timezone"));
  fireEvent.change(screen.getByLabelText("Search timezones"), {
    target: { value: "New_York" },
  });
  await user.click(screen.getByRole("option", { name: "America/New_York" }));
  mocks.settings.mockReturnValue({ data: { timezone: "America/Chicago" } });
  view.rerender(<AutomationEditor onClose={vi.fn()} onSecrets={vi.fn()} />);
  expect(screen.getByLabelText("IANA timezone")).toHaveTextContent(
    "America/New_York",
  );
});
it.each(["active", "disabled"] as const)(
  "%s automation supports pause or resume and run-now controls",
  async (status) => {
    const user = userEvent.setup();
    mocks.triggers.mockReturnValue({
      data: { triggers: [{ ...trigger, status }] },
    });
    render(<AutomationsPage />);
    await user.click(
      screen.getAllByRole("button", {
        name: "Actions for Morning summary",
      })[0]!,
    );
    if (status === "disabled")
      expect(screen.getByRole("menuitem", { name: "Run now" })).toHaveAttribute(
        "aria-disabled",
        "true",
      );
    await user.click(
      screen.getByRole("menuitem", {
        name: status === "active" ? "Pause" : "Resume",
      }),
    );
    expect(mocks.update).toHaveBeenCalledWith({
      id: trigger.id,
      data: { status: status === "active" ? "disabled" : "active" },
    });
    if (status === "active") {
      await user.click(
        screen.getAllByRole("button", {
          name: "Actions for Morning summary",
        })[0]!,
      );
      await user.click(screen.getByRole("menuitem", { name: "Run now" }));
      expect(mocks.run).toHaveBeenCalledWith(trigger.id);
    }
  },
);
it("edits a schedule without changing its source", async () => {
  const user = userEvent.setup();
  render(
    <AutomationEditor row={trigger} onClose={vi.fn()} onSecrets={vi.fn()} />,
  );
  expect(screen.getByLabelText("Source")).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Instruction"), {
    target: { value: "Summarise unread notifications" },
  });
  await user.click(screen.getByRole("button", { name: "Save automation" }));
  await waitFor(() =>
    expect(mocks.update).toHaveBeenCalledWith({
      id: trigger.id,
      data: expect.objectContaining({
        delivery: expect.objectContaining({
          instruction: "Summarise unread notifications",
        }),
      }),
    }),
  );
});
it("shows history thread links, pages older runs and confirms deletion", async () => {
  const user = userEvent.setup();
  mocks.triggers.mockReturnValue({ data: { triggers: [trigger] } });
  mocks.history.mockReturnValue({
    data: {
      pages: [
        {
          runs: [
            {
              id: "run",
              outcome: "completed",
              scheduled_at: trigger.created_at,
              reason: null,
              duration_ms: 2000,
              thread_id: "thread-1",
            },
            {
              id: "summary",
              outcome: "skipped",
              scheduled_at: trigger.created_at,
              reason: "missed_window",
              duration_ms: null,
              thread_id: null,
              missed: {
                count: 864,
                from: "2026-09-28T00:00:00Z",
                through: "2026-09-30T23:55:00Z",
              },
            },
          ],
        },
      ],
    },
    hasNextPage: true,
    fetchNextPage: mocks.older,
  });
  render(<AutomationsPage />);
  await user.click(
    screen.getAllByRole("button", { name: "Morning summary" })[0]!,
  );
  expect(screen.getByRole("link", { name: "Open thread" })).toHaveAttribute(
    "href",
    "/assistant?c=thread-1",
  );
  expect(
    screen.getByText(/Missed 864 occurrences between/),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Load older runs" }));
  expect(mocks.older).toHaveBeenCalled();
  await user.keyboard("{Escape}");
  await user.click(
    screen.getAllByRole("button", { name: "Actions for Morning summary" })[0]!,
  );
  await user.click(screen.getByRole("menuitem", { name: "Delete" }));
  expect(mocks.remove).not.toHaveBeenCalled();
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "Delete" }),
  );
  expect(mocks.remove).toHaveBeenCalledWith(trigger.id);
});
it("shows preference validation errors and keeps invalid timezones unsaved", async () => {
  mocks.settings.mockReturnValue({ data: { timezone: "Mars/Olympus" } });
  const user = userEvent.setup();
  render(<AutomationPreferences />);
  await user.click(
    screen.getByRole("button", { name: "Timezone and automation budgets" }),
  );

  await user.click(screen.getByRole("button", { name: "Save preferences" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("IANA timezone");
  expect(mocks.saveSettings).not.toHaveBeenCalled();
});

it("loads a private setup from the validated watch id and shows its warning", async () => {
  const user = userEvent.setup();
  const setup = "80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288";
  mocks.search.mockReturnValue({ setup });
  mocks.setup.mockReturnValue({ isPending: true });
  const { rerender } = render(<AutomationsPage />);
  expect(mocks.setup).toHaveBeenCalledWith(setup);
  expect(screen.getByLabelText("Loading setup")).toBeInTheDocument();
  mocks.setup.mockReturnValue({
    data: {
      label: "Private webhook",
      instruction: "Read incoming issues",
      agent_id: "nyxbot",
      confirmation_policy: "changes",
    },
  });
  rerender(<AutomationsPage />);
  expect(screen.getByLabelText("Instruction")).toHaveValue(
    "Read incoming issues",
  );
  expect(screen.getByLabelText("Confirm webhook actions")).toHaveTextContent(
    "Every changing action",
  );
  expect(screen.getByLabelText("Thread")).toHaveTextContent(
    "Default (dedicated thread)",
  );
  expect(
    screen.queryByText(/can influence later owner turns/),
  ).not.toBeInTheDocument();
  await user.click(screen.getByLabelText("Thread"));
  await user.click(screen.getByRole("option", { name: "Home thread" }));
  expect(
    screen.getByText(/can influence later owner turns/),
  ).toBeInTheDocument();
  await user.click(screen.getByLabelText("Confirm webhook actions"));
  await user.click(
    screen.getByRole("option", { name: "Only destructive actions" }),
  );
  expect(screen.getByText(/account without confirmation/)).toBeInTheDocument();
});

it("keeps expired setup instructions unavailable", () => {
  mocks.search.mockReturnValue({
    setup: "80c7e6d9-41d5-48c3-bfd7-2bf9c92fa288",
  });
  mocks.setup.mockReturnValue({ error: new Error("Setup link expired") });
  render(<AutomationsPage />);
  expect(screen.getByText("Setup link expired")).toBeInTheDocument();
  expect(screen.queryByLabelText("Instruction")).not.toBeInTheDocument();
});
