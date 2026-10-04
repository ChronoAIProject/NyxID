import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import {
  fireEvent,
  render as rtlRender,
  screen,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { MachineToolCard } from "./machine-tool-card";
import { GroupMessageRow } from "./nyxbot-group-view";
import { assistantGroupMessageSchema } from "@/schemas/assistant-nyxagent";
import { assistantHttp, assistantJson } from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-client";
import type { MachineReceipt } from "@/schemas/machine-activity";
vi.mock("@/lib/assistant/assistant-http", () => ({
  assistantHttp: vi.fn(),
  assistantJson: vi.fn(),
}));
function render(node: ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return rtlRender(
    <QueryClientProvider client={client}>{node}</QueryClientProvider>,
  );
}
const receipt: MachineReceipt = {
  operation_id: "operation",
  node_id: "machine",
  agent_id: "agent",
  action: "command.exec",
  status: "completed",
  job_id: "job",
  exit_code: 0,
  bytes: null,
  duration_ms: 10,
  error_code: null,
  screenshot_id: null,
  preview_id: null,
  preview_enabled: false,
};
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(assistantJson).mockImplementation(async (_url, options) => ({
    enabled: options?.method === "PUT",
  }));
});
afterEach(cleanup);
it("loads current consent instead of trusting an old receipt", async () => {
  vi.mocked(assistantJson).mockResolvedValue({ enabled: true });
  render(<MachineToolCard receipt={receipt} conversationId="nyxa-owner" />);
  fireEvent.click(screen.getByText("Command excerpt preference"));
  expect(
    await screen.findByRole("button", { name: "Stop saving excerpts" }),
  ).toBeEnabled();
  expect(assistantJson).toHaveBeenCalledWith(
    "/assistant/nyxagent/conversations/nyxa-owner/machine-preview-policy",
  );
});
it("shows the typed job result and changes only human preview consent", async () => {
  render(<MachineToolCard receipt={receipt} conversationId="nyxa-owner" />);
  expect(screen.getByText("Exit 0")).toBeInTheDocument();
  expect(assistantHttp).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Command excerpt preference"));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Save future excerpts" }),
    ).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Save future excerpts" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Stop saving excerpts" }),
    ).toBeInTheDocument(),
  );
  expect(assistantJson).toHaveBeenCalledWith(
    "/assistant/nyxagent/conversations/nyxa-owner/machine-preview-policy",
    { method: "PUT", body: { enabled: true } },
  );
});
it("fetches encrypted excerpts only on reveal and renders plain text", async () => {
  vi.mocked(assistantHttp).mockResolvedValue(
    new Response("<script>private-output</script>"),
  );
  render(
    <MachineToolCard
      receipt={{ ...receipt, preview_id: "preview" }}
      conversationId="nyxa-owner"
    />,
  );
  expect(assistantHttp).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Show saved excerpt" }));
  expect(
    await screen.findByText("<script>private-output</script>"),
  ).toBeInTheDocument();
  expect(document.querySelector("script")).toBeNull();
  expect(assistantHttp).toHaveBeenCalledWith(
    "/assistant/nyxagent/conversations/nyxa-owner/attachments/preview",
    expect.objectContaining({ signal: expect.any(AbortSignal) }),
  );
});
it("shows a clear retention placeholder when a saved excerpt expires", async () => {
  vi.mocked(assistantHttp).mockRejectedValue(
    new ApiError(410, {
      error: "attachment_expired",
      error_code: 12101,
      message: "expired",
    }),
  );
  render(
    <MachineToolCard
      receipt={{ ...receipt, preview_id: "expired" }}
      conversationId="nyxa-owner"
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Show saved excerpt" }));
  expect(
    await screen.findByText(/expired per retention policy/),
  ).toBeInTheDocument();
});
it("group metadata cards expose neither private fetches nor preview consent", () => {
  render(
    <MachineToolCard
      receipt={{ ...receipt, preview_id: "private-sentinel" }}
    />,
  );
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(assistantHttp).not.toHaveBeenCalled();
  expect(screen.queryByText("private-sentinel")).not.toBeInTheDocument();
});
it("shows file byte counts and existing screenshot expiry without fetching pixels", () => {
  render(
    <MachineToolCard
      receipt={{
        ...receipt,
        action: "file.write",
        bytes: 123,
        screenshot_id: "image",
      }}
      images={[
        {
          id: "image",
          endpoint: "/private/image",
          expired: true,
          contentType: "image/png",
          label: "Machine screenshot",
        },
      ]}
    />,
  );
  expect(screen.getByText("123 bytes")).toBeInTheDocument();
  expect(screen.getByText(/expired per retention policy/)).toBeInTheDocument();
  expect(assistantHttp).not.toHaveBeenCalled();
});

it.each(["agent", "notice"])(
  "renders published %s group receipts without private controls",
  (role) => {
    const message = assistantGroupMessageSchema.parse({
      id: "message",
      group_id: "group",
      seq: 1,
      role,
      text: "Finished",
      created_at: "2026-10-03T00:00:00Z",
      agent: { id: "agent", name: "worker", kind: "specialist" },
      activities: [
        {
          id: "activity",
          label: "nyx__machine_exec",
          status: "completed",
          started_at: "2026-10-03T00:00:00Z",
          ended_at: null,
          machine: receipt,
        },
      ],
    });
    render(
      <GroupMessageRow
        groupId="group"
        message={message}
        names={[]}
        onOpenAgent={vi.fn()}
      />,
    );
    expect(screen.getByText("Exit 0")).toBeInTheDocument();
    expect(
      screen.queryByText("Command excerpt preference"),
    ).not.toBeInTheDocument();
    expect(assistantHttp).not.toHaveBeenCalled();
  },
);

it("renders the resolved machine name and keeps raw correlation IDs collapsed", () => {
  render(
    <MachineToolCard
      receipt={{
        ...receipt,
        node_id: "22222222-2222-4222-8222-222222222222",
        machine_name: "Work MacBook Air",
      }}
    />,
  );
  expect(
    screen.getByText("Work MacBook Air · Shared workspace and browser"),
  ).toBeInTheDocument();
  expect(screen.queryByText(/Machine 22222222/)).not.toBeInTheDocument();
  expect(
    screen.getByText("22222222-2222-4222-8222-222222222222"),
  ).not.toBeVisible();
});
it.each([undefined, null, " "])(
  "uses the short machine ID only when its name is missing (%s)",
  (machine_name) => {
    render(
      <MachineToolCard
        receipt={{
          ...receipt,
          node_id: "22222222-2222-4222-8222-222222222222",
          machine_name,
        }}
      />,
    );
    expect(
      screen.getByText("Machine 22222222 · Shared workspace and browser"),
    ).toBeInTheDocument();
  },
);

it("labels separated receipts without claiming shared sessions or full isolation", () => {
  render(<MachineToolCard receipt={{ ...receipt, machine_name: "Linux VM", context_mode: "separated" }} />);
  expect(screen.getByText("Linux VM · Separate workspace and browser for this agent")).toBeInTheDocument();
  expect(screen.queryByText(/Shared workspace and browser/)).not.toBeInTheDocument();
  expect(screen.queryByText(/isolated/i)).not.toBeInTheDocument();
});
