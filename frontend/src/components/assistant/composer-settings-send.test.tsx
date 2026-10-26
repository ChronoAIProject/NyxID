vi.mock("./composer-editor", () => import("./composer-editor.test-support"));
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NyxAgentComposer } from "./nyxagent-composer";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { nyxAgentConversationSchema } from "@/schemas/assistant-nyxagent";
import { createUploadDraft } from "@/lib/assistant/uploads";

const id = `nyxa-${"a".repeat(32)}`;
vi.mock("@/lib/assistant/uploads", async (original) => ({
  ...(await original<typeof import("@/lib/assistant/uploads")>()),
  createUploadDraft: vi.fn(async () => `nyxa-${"a".repeat(32)}`),
  uploadFile: vi.fn(async () => ({
    id: "file-1",
    label: "notes.txt",
    media_type: "text/plain",
    size_bytes: 4,
  })),
}));
const onSend = vi.fn(async () => {});
let client: QueryClient;

beforeEach(() => {
  useAuthStore.getState().setUser(null);
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  onSend.mockClear();
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  client.setQueryData(nyxBotQueryKeys.models(undefined), [
    { id: "nyxagent/chat", label: "Default model", available: true },
    { id: "nyxagent/research", label: "Research", available: true },
  ]);
  client.setQueryData(nyxBotQueryKeys.settings(undefined), {
    access_mode: "nyxmode",
  });
});
afterEach(() => {
  client.clear();
  vi.restoreAllMocks();
});

it("opens voice settings from the chevron without starting a call or creating a draft", async () => {
  const onVoiceSettings = vi.fn();
  const onVoice = vi.fn();
  vi.mocked(createUploadDraft).mockClear();
  render(<QueryClientProvider client={client}><NyxAgentComposer
    active={false} sending={false} ownerUserId="owner" draftKey="screen:nyxagent:assistant"
    scope={{ kind: "conversations" }} onSend={onSend} onStop={async () => {}}
    onVoice={onVoice} onVoiceSettings={onVoiceSettings}
  /></QueryClientProvider>);
  const user = userEvent.setup({ skipHover: true });
  await user.click(screen.getByRole("button", { name: /Thinking and permissions/ }));
  await user.click(screen.getByRole("menuitem", { name: "Voice settings" }));
  expect(onVoiceSettings).toHaveBeenCalledOnce();
  expect(onVoice).not.toHaveBeenCalled();
  expect(createUploadDraft).not.toHaveBeenCalled();
});

async function draftWithUpload() {
  render(
    <QueryClientProvider client={client}>
      <NyxAgentComposer
        active={false}
        sending={false}
        ownerUserId="owner"
        draftKey="screen:nyxagent:assistant"
        scope={{ kind: "conversations" }}
        onSend={onSend}
        onStop={async () => {}}
      />
    </QueryClientProvider>,
  );
  const user = userEvent.setup({ skipHover: true });
  await user.click(
    screen.getByRole("button", { name: /Thinking and permissions/ }),
  );
  await user.click(screen.getByRole("menuitemcheckbox", { name: "Thinking mode" }));
  await user.keyboard("{Escape}");
  fireEvent.change(screen.getByLabelText("Choose attachments"), {
    target: {
      files: [new File(["test"], "notes.txt", { type: "text/plain" })],
    },
  });
  await screen.findByText("Ready");
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Read my notes" },
  });
}

it("shows the inherited specialist model and sends an explicit change from it", async () => {
  render(
    <QueryClientProvider client={client}>
      <NyxAgentComposer
        active={false}
        sending={false}
        ownerUserId="owner"
        draftKey="screen:nyxagent:agent:specialist"
        scope={{ kind: "conversations", agentId: "specialist" }}
        agentModel="nyxagent/research"
        onSend={onSend}
        onStop={async () => {}}
      />
    </QueryClientProvider>,
  );
  const user = userEvent.setup({ skipHover: true });
  await user.click(screen.getByRole("button", { name: /Thinking and permissions/ }));
  const thinking = screen.getByRole("menuitemcheckbox", { name: "Thinking mode" });
  expect(thinking).toHaveAttribute("aria-checked", "true");
  await user.click(thinking);
  await user.keyboard("{Escape}");
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "Answer quickly" } });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledWith("Answer quickly", undefined, "nyxagent/chat"));
});

it("applies the selected model to an upload draft before sending and blocks duplicate submission", async () => {
  let complete!: () => void;
  const gate = new Promise<void>((resolve) => {
    complete = resolve;
  });
  const change = vi
    .spyOn(nyxBotApi, "changeModel")
    .mockImplementation(async () => {
      await gate;
      return nyxAgentConversationSchema.parse({
        id,
        model: "nyxagent/research",
        title: "Draft",
        created_at: "2026-10-15T00:00:00Z",
        last_message_at: "2026-10-15T00:00:00Z",
        message_count: 0,
        active_turn: null,
        context_reset_at: null,
      });
    });
  await draftWithUpload();
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() =>
    expect(change).toHaveBeenCalledWith(id, "nyxagent/research"),
  );
  expect(onSend).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: "Send message" })).not.toBeInTheDocument();
  await act(async () => complete());
  await waitFor(() =>
    expect(onSend).toHaveBeenCalledWith(
      "Read my notes",
      expect.objectContaining({
        conversationId: id,
        attachmentIds: ["file-1"],
      }),
      "nyxagent/research",
    ),
  );
});

it("keeps text and attachments when the selected model is rejected before send", async () => {
  vi.spyOn(nyxBotApi, "changeModel").mockRejectedValue(
    new Error("Model unavailable"),
  );
  await draftWithUpload();
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() =>
    expect(screen.getByRole("textbox")).toHaveValue("Read my notes"),
  );
  expect(await screen.findByText("Ready")).toBeVisible();
  expect(onSend).not.toHaveBeenCalled();
});
