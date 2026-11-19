const realKeysFetch = vi.hoisted(() => ({ enabled: false }));
vi.mock("@/hooks/use-keys", async (original) => {
  const hooks = await original<typeof import("@/hooks/use-keys")>();
  return { ...hooks, useKeys: () => {
    const query = hooks.useKeys();
    return realKeysFetch.enabled ? query : { ...query, refetch: vi.fn() };
  } };
});
vi.mock("./composer-editor", () => import("./composer-editor.test-support"));
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { onlineManager, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { toast } from "sonner";
import { NyxAgentComposer } from "./nyxagent-composer";
import { markAdmitted } from "@/lib/assistant/send-admission";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { nyxAgentConversationSchema } from "@/schemas/assistant-nyxagent";
import { createUploadDraft, uploadFile } from "@/lib/assistant/uploads";
import { api } from "@/lib/api-client";
import type { User } from "@/types/api";
import type { KeyInfo } from "@/types/keys";

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
const originalClearDraft = useAssistantDraftStore.getState().clearDraft;
let client: QueryClient;

beforeEach(() => {
  realKeysFetch.enabled = false;
  useAuthStore.getState().setUser(null);
  transitionAssistantIdentity("owner");
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
  onlineManager.setOnline(true);
  vi.restoreAllMocks();
  // Zustand copies actions into new snapshots, beyond spyOn's original target.
  useAssistantDraftStore.setState({ clearDraft: originalClearDraft });
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

it("rechecks live connection state after pending model setup and retains the draft and upload", async () => {
  const key = { id: "selected", label: "Work account", slug: "work", is_active: true, status: "active" } as KeyInfo;
  client.setQueryData(["keys", "list", undefined], [key]);
  let complete!: () => void;
  const pending = new Promise<void>((resolve) => { complete = resolve; });
  const change = vi.spyOn(nyxBotApi, "changeModel").mockImplementation(async () => {
    await pending;
    return nyxAgentConversationSchema.parse({
      id, model: "nyxagent/research", title: "Draft", created_at: "2026-10-15T00:00:00Z",
      last_message_at: "2026-10-15T00:00:00Z", message_count: 0,
      active_turn: null, context_reset_at: null,
    });
  });
  await draftWithUpload();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Add context" }));
  await user.click(screen.getByRole("menuitem", { name: "Use a connection" }));
  await user.click(screen.getByRole("button", { name: /Work account/ }));
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(change).toHaveBeenCalledOnce());
  act(() => client.setQueryData(["keys", "list", undefined], [{ ...key, credential_missing: true }]));
  await waitFor(() => expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("Credential missing"));
  await act(async () => complete());
  await waitFor(() => expect(screen.getByRole("textbox")).toHaveValue("Read my notes"));
  expect(screen.getByText("Ready")).toBeVisible();
  expect(onSend).not.toHaveBeenCalled();
});


it.each([
  { modalities: ["text"], warns: true },
  { modalities: ["text", "image"], warns: false },
  { modalities: undefined, warns: false },
])("warns without blocking an image send only for known text-only models ($warns)", async ({ modalities, warns }) => {
  client.setQueryData(nyxBotQueryKeys.models(undefined), [
    { id: "nyxagent/chat", label: "Default model", available: true, input_modalities: modalities },
  ]);
  vi.mocked(uploadFile).mockResolvedValueOnce({ id: "photo", label: "photo.png", content_type: "image/png", size: 4, origin: "user_upload" });
  render(<QueryClientProvider client={client}><NyxAgentComposer
    active={false} sending={false} ownerUserId="owner" draftKey="screen:nyxagent:assistant"
    scope={{ kind: "conversations" }} onSend={onSend} onStop={async () => {}}
  /></QueryClientProvider>);
  expect(screen.queryByText(/This model can’t view images/)).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Choose attachments"), { target: { files: [new File(["test"], "photo.png", { type: "image/png" })] } });
  await screen.findByText("Ready");
  if (warns) expect(screen.getByText(/This model can’t view images/)).toBeVisible();
  else expect(screen.queryByText(/This model can’t view images/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ attachmentIds: ["photo"] }), undefined));
});

it("keeps the draft store and upload attached through refused pointer and Enter sends", async () => {
  const key = { id: "selected", label: "Work account", slug: "work", is_active: true, status: "active" } as KeyInfo;
  client.setQueryData(["keys", "list", undefined], [key]);
  await draftWithUpload();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Add context" }));
  await user.click(screen.getByRole("menuitem", { name: "Use a connection" }));
  await user.click(screen.getByRole("button", { name: /Work account/ }));
  act(() => client.setQueryData(["keys", "list", undefined], [{ ...key, credential_missing: true }]));
  await waitFor(() => expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("Credential missing"));
  await waitFor(() => expect(Object.values(useAssistantDraftStore.getState().drafts).some((draft) => draft.text === "Read my notes")).toBe(true));
  const before = useAssistantDraftStore.getState().drafts;
  const clear = vi.spyOn(useAssistantDraftStore.getState(), "clearDraft");
  const detachments: boolean[] = [];
  const observer = new MutationObserver(() => detachments.push(Boolean(screen.queryByText("Ready"))));
  observer.observe(document.body, { childList: true, subtree: true });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", code: "Enter" });
  await act(async () => {});
  observer.disconnect();
  expect(clear).not.toHaveBeenCalled();
  expect(useAssistantDraftStore.getState().drafts).toBe(before);
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
  expect(detachments).not.toContain(false);
  expect(onSend).not.toHaveBeenCalled();
});

it("A2: propagates an admitted interruption so the original upload is consumed", async () => {
  const key = { id: "selected", label: "Work account", slug: "work", is_active: true, status: "active" } as KeyInfo;
  client.setQueryData(["keys", "list", undefined], [key]);
  const admitted = markAdmitted(new Error("stream interrupted"));
  onSend.mockRejectedValueOnce(admitted as never);
  await draftWithUpload();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Add context" }));
  await user.click(screen.getByRole("menuitem", { name: "Use a connection" }));
  await user.click(screen.getByRole("button", { name: /Work account/ }));
  // Avoid model HTTP setup; this test isolates the admitted send boundary.
  vi.spyOn(nyxBotApi, "changeModel").mockResolvedValue({} as never);
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledOnce());
  await act(async () => {});
  expect(screen.getByRole("textbox")).toHaveValue("");
  expect(screen.queryByText("Ready")).not.toBeInTheDocument();
  expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "Next message" } });
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledTimes(2));
  expect(onSend).toHaveBeenLastCalledWith("Next message", undefined, "nyxagent/research");
});

const liveKey = { id: "selected", label: "Work account", slug: "work", is_active: true, status: "active" } as KeyInfo;
const liveList = ["keys", "list", "owner"];
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function realKeys() {
  realKeysFetch.enabled = true;
  useAuthStore.setState({ user: { id: "owner" } as User });
  client.setQueryData(liveList, [liveKey]);
  client.setQueryData(nyxBotQueryKeys.models("owner"), client.getQueryData(nyxBotQueryKeys.models(undefined)));
  client.setQueryData(nyxBotQueryKeys.settings("owner"), { access_mode: "nyxmode" });
  vi.spyOn(nyxBotApi, "settings").mockResolvedValue({ access_mode: "nyxmode" } as never);
  let next = () => Promise.resolve({ keys: client.getQueryData<readonly KeyInfo[]>(liveList) ?? [] });
  const get = vi.spyOn(api, "get").mockImplementation((() => next()) as typeof api.get);
  return { get, hold: () => {
    const gate = deferred<{ keys: readonly KeyInfo[] }>();
    next = () => gate.promise;
    return gate;
  } };
}
async function workPicker() {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Add context" }));
  await user.click(screen.getByRole("menuitem", { name: "Use a connection" }));
  return user;
}
async function chooseWork() {
  const user = await workPicker();
  await user.click(screen.getByRole("button", { name: /Work account/ }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
}

it("A1: reads a synchronous cache revocation when model setup resolves in the same act", async () => {
  realKeys();
  const gate = deferred<void>();
  const change = vi.spyOn(nyxBotApi, "changeModel").mockImplementation(async () => { await gate.promise; return {} as never; });
  await draftWithUpload();
  await chooseWork();
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(change).toHaveBeenCalledOnce());
  await act(async () => {
    client.setQueryData(liveList, [{ ...liveKey, credential_missing: true }]);
    gate.resolve();
  });
  expect(onSend).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
  await waitFor(() => expect(useAssistantDraftStore.getState().getDraft("screen:nyxagent:assistant")).toBe("Read my notes"));
  act(() => client.setQueryData(liveList, [liveKey]));
  await waitFor(() => expect(screen.getByRole("group", { name: "Connection for this message" })).not.toHaveTextContent("Unavailable"));
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledExactlyOnceWith("/service selected\nRead my notes", expect.objectContaining({ attachmentIds: ["file-1"] }), "nyxagent/research"));
});

it("A1: refuses pointer and Enter before clearing text or uploads while an opening GET is held", async () => {
  const transport = realKeys();
  vi.spyOn(nyxBotApi, "changeModel").mockResolvedValue({} as never);
  await draftWithUpload();
  await chooseWork();
  await waitFor(() => expect(useAssistantDraftStore.getState().getDraft("screen:nyxagent:assistant")).toBe("Read my notes"));
  const gate = transport.hold();
  const user = await workPicker();
  await user.keyboard("{Escape}");
  const chip = screen.getByRole("group", { name: "Connection for this message" });
  expect(chip).toHaveTextContent("Checking availability…");
  expect(chip).not.toHaveTextContent("Unavailable");
  expect(within(chip).getByRole("status")).not.toHaveTextContent("Checking");
  const clear = vi.spyOn(useAssistantDraftStore.getState(), "clearDraft");
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", code: "Enter" });
  await act(async () => {});
  expect(onSend).not.toHaveBeenCalled();
  expect(clear).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
  expect(useAssistantDraftStore.getState().getDraft("screen:nyxagent:assistant")).toBe("Read my notes");
  expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("Checking availability…");
  expect(within(chip).getByRole("status")).toHaveTextContent("Checking availability…");
  await act(async () => gate.resolve({ keys: [{ ...liveKey, credential_missing: true }] }));
  await waitFor(() => expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("Credential missing"));
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  expect(onSend).not.toHaveBeenCalled();
  const usable = transport.hold();
  await workPicker();
  await act(async () => usable.resolve({ keys: [liveKey] }));
  await user.keyboard("{Escape}");
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledExactlyOnceWith("/service selected\nRead my notes", expect.objectContaining({ attachmentIds: ["file-1"] }), "nyxagent/research"));
});

it.each(["unavailable", "available", "error", "closed"])("A1: held picker activation settles as %s", async (outcome) => {
  const transport = realKeys();
  await draftWithUpload();
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("idle"));
  const gate = transport.hold();
  const user = await workPicker();
  const option = screen.getByRole("button", { name: /Work account/ });
  await user.click(option);
  expect(screen.getByRole("dialog")).toBeVisible();
  expect(option).toHaveAttribute("aria-busy", "true");
  expect(option).toHaveAccessibleDescription("Checking availability…");
  if (outcome === "closed") await user.keyboard("{Escape}");
  await act(async () => {
    if (outcome === "error") gate.reject(new Error("keys unavailable"));
    else gate.resolve({ keys: [{ ...liveKey, credential_missing: outcome === "unavailable" }] });
  });
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("idle"));
  if (outcome === "available") {
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getAllByRole("group", { name: "Connection for this message" })).toHaveLength(1);
  } else {
    expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
    if (outcome === "unavailable") expect(option).toHaveAccessibleDescription("Credential missing");
    if (outcome === "error") expect(await screen.findByRole("button", { name: "Retry" })).toBeVisible();
    if (outcome === "closed") expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  }
});

it.each(["fetching", "error", "missing", "authority"])("A1: rejects %s after model setup without losing the ready upload", async (state) => {
  const transport = realKeys();
  const model = deferred<void>();
  const change = vi.spyOn(nyxBotApi, "changeModel").mockImplementation(async () => { await model.promise; return {} as never; });
  await draftWithUpload(); await chooseWork();
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(change).toHaveBeenCalledOnce());
  await act(async () => {
    if (state === "fetching") {
      transport.hold();
      void client.refetchQueries({ queryKey: liveList, exact: true });
    } else if (state === "error") {
      client.getQueryCache().find({ queryKey: liveList, exact: true })!.setState({ status: "error", error: new Error("keys failed") });
    } else if (state === "missing") client.setQueryData(liveList, []);
    else useAuthStore.setState({ user: { id: "other" } as User });
    model.resolve();
  });
  expect(onSend).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
});

it("A1: sends without an override while a keys GET is pending", async () => {
  const transport = realKeys();
  vi.spyOn(nyxBotApi, "changeModel").mockResolvedValue({} as never);
  await draftWithUpload();
  transport.hold();
  const user = await workPicker();
  await user.keyboard("{Escape}");
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledExactlyOnceWith("Read my notes", expect.objectContaining({ attachmentIds: ["file-1"] }), "nyxagent/research"));
});

it("A1: clears checking when an identical refresh settles before React observes fetching", async () => {
  const transport = realKeys();
  await draftWithUpload(); await chooseWork();
  const gate = transport.hold();
  await act(async () => {
    const refresh = client.refetchQueries({ queryKey: liveList, exact: true });
    fireEvent.click(screen.getByRole("button", { name: "Send message" }));
    gate.resolve({ keys: [liveKey] });
    await refresh;
  });
  expect(onSend).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
  await waitFor(() => expect(screen.getByRole("group", { name: "Connection for this message" })).not.toHaveTextContent("Checking availability…"));
});

it.each(["revoked", "usable"])("R1: holds offline activation through paused and fetching until %s settlement", async (outcome) => {
  const transport = realKeys();
  await draftWithUpload();
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("idle"));
  const gate = transport.hold();
  const gets = transport.get.mock.calls.filter(([path]) => path === "/keys").length;
  act(() => onlineManager.setOnline(false));
  const user = await workPicker();
  expect(client.getQueryState(liveList)?.fetchStatus).toBe("paused");
  const option = screen.getByRole("button", { name: /Work account/ });
  await user.click(option);
  expect(screen.getByRole("dialog")).toBeVisible();
  expect(option).toHaveAttribute("aria-busy", "true");
  expect(option).toHaveAccessibleDescription("Checking availability…");
  expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
  expect(transport.get.mock.calls.filter(([path]) => path === "/keys")).toHaveLength(gets);
  act(() => onlineManager.setOnline(true));
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("fetching"));
  expect(option).toHaveAttribute("aria-busy", "true");
  expect(screen.getByRole("dialog")).toBeVisible();
  await act(async () => gate.resolve({ keys: [{ ...liveKey, credential_missing: outcome === "revoked" }] }));
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("idle"));
  expect(transport.get.mock.calls.filter(([path]) => path === "/keys")).toHaveLength(gets + 1);
  if (outcome === "revoked") {
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(option).toHaveAccessibleDescription("Credential missing");
    expect(screen.queryByRole("group", { name: "Connection for this message" })).not.toBeInTheDocument();
  } else {
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getAllByRole("group", { name: "Connection for this message" })).toHaveLength(1);
    expect(screen.getByRole("group", { name: "Connection for this message" })).toHaveTextContent("Work account");
  }
  expect(onSend).not.toHaveBeenCalled();
});

it("R1: paused refresh refuses pointer and Enter before clearing, then resumes revoked and retries usable once", async () => {
  const transport = realKeys();
  vi.spyOn(nyxBotApi, "changeModel").mockResolvedValue({} as never);
  await draftWithUpload(); await chooseWork();
  await waitFor(() => expect(useAssistantDraftStore.getState().getDraft("screen:nyxagent:assistant")).toBe("Read my notes"));
  const gate = transport.hold();
  act(() => onlineManager.setOnline(false));
  const user = await workPicker();
  expect(client.getQueryState(liveList)?.fetchStatus).toBe("paused");
  await user.keyboard("{Escape}");
  const chip = screen.getByRole("group", { name: "Connection for this message" });
  expect(chip).toHaveTextContent("Checking availability…");
  expect(chip).not.toHaveTextContent("Unavailable");
  expect(within(chip).getByRole("status")).not.toHaveTextContent("Checking");
  const clear = vi.spyOn(useAssistantDraftStore.getState(), "clearDraft");
  const error = vi.spyOn(toast, "error").mockReturnValue("refused");
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", code: "Enter" });
  await act(async () => {});
  expect(onSend).not.toHaveBeenCalled();
  expect(clear).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
  expect(useAssistantDraftStore.getState().getDraft("screen:nyxagent:assistant")).toBe("Read my notes");
  expect(within(chip).getByRole("status")).toHaveTextContent("Checking availability…");
  expect(error).toHaveBeenLastCalledWith("Checking the selected connection. Send again when it's ready.", {
    id: "composer-connection-unavailable", description: "Checking availability…",
  });
  act(() => onlineManager.setOnline(true));
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("fetching"));
  expect(within(chip).getByRole("status")).toHaveTextContent("Checking availability…");
  await act(async () => gate.resolve({ keys: [{ ...liveKey, credential_missing: true }] }));
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("idle"));
  await waitFor(() => expect(chip).toHaveTextContent("Credential missing"));
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", code: "Enter" });
  expect(onSend).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
  const usable = transport.hold();
  await workPicker();
  await act(async () => usable.resolve({ keys: [liveKey] }));
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("idle"));
  await user.keyboard("{Escape}");
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledExactlyOnceWith("/service selected\nRead my notes", expect.objectContaining({ attachmentIds: ["file-1"] }), "nyxagent/research"));
});

it("R1: reopening the picker joins the paused refresh without another refetch", async () => {
  realKeys(); await draftWithUpload();
  await waitFor(() => expect(client.getQueryState(liveList)?.fetchStatus).toBe("idle"));
  const fetch = vi.spyOn(client.getQueryCache().find({ queryKey: liveList, exact: true })!, "fetch");
  act(() => onlineManager.setOnline(false));
  const user = await workPicker();
  expect(client.getQueryState(liveList)?.fetchStatus).toBe("paused");
  expect(fetch).toHaveBeenCalledOnce();
  await user.keyboard("{Escape}");
  await workPicker();
  expect(client.getQueryState(liveList)?.fetchStatus).toBe("paused");
  expect(fetch).toHaveBeenCalledOnce();
});

it("R1: refuses a paused query after model setup and restores the original ready upload", async () => {
  realKeys();
  const model = deferred<void>();
  const change = vi.spyOn(nyxBotApi, "changeModel").mockImplementation(async () => { await model.promise; return {} as never; });
  await draftWithUpload(); await chooseWork();
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(change).toHaveBeenCalledOnce());
  await act(async () => {
    onlineManager.setOnline(false);
    void client.refetchQueries({ queryKey: liveList, exact: true });
    expect(client.getQueryState(liveList)?.fetchStatus).toBe("paused");
    model.resolve();
  });
  expect(onSend).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox")).toHaveValue("Read my notes");
  expect(screen.getByText("Ready")).toBeVisible();
  await waitFor(() => expect(useAssistantDraftStore.getState().getDraft("screen:nyxagent:assistant")).toBe("Read my notes"));
});

it("R1: dispatches a no-override send once while the refresh remains paused", async () => {
  realKeys();
  vi.spyOn(nyxBotApi, "changeModel").mockResolvedValue({} as never);
  await draftWithUpload();
  act(() => onlineManager.setOnline(false));
  const user = await workPicker();
  expect(client.getQueryState(liveList)?.fetchStatus).toBe("paused");
  await user.keyboard("{Escape}");
  fireEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() => expect(onSend).toHaveBeenCalledExactlyOnceWith("Read my notes", expect.objectContaining({ attachmentIds: ["file-1"] }), "nyxagent/research"));
  expect(client.getQueryState(liveList)?.fetchStatus).toBe("paused");
});
