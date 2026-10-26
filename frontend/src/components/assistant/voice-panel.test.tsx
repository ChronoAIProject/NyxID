import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { PreparedMicrophone } from "@/lib/microphone-capture";
import { voiceOption, voicePreferences } from "@/test/voice-fixture";
import { QueryClient } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import type { VoicePreferences } from "@/schemas/assistant-voice";
import { VoicePanel } from "./voice-panel";
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/api-client")>(),
  api: { put: vi.fn().mockResolvedValue({}) },
}));
const mock = vi.hoisted(() => ({
  queryClient: null as QueryClient | null,
  save: vi.fn(),
  refetch: vi.fn(),
  optionsError: false,
  starting: false,
  error: null as string | null,
  connected: true,
  options: [] as unknown[],
  end: vi.fn(),
  start: vi.fn(),
  mute: vi.fn(),
  speaker: vi.fn(),
  hold: vi.fn(),
  stop: vi.fn(),
  rejoin: vi.fn(),
  conflictSession: null as null | {
    id: string;
    resumable: boolean;
    control_revision: number;
  },
  tasks: [] as Array<Record<string, unknown>>,
}));
vi.mock("@/lib/assistant/nyxbot-api", () => ({
  nyxBotApi: { updateSettings: mock.save },
}));
vi.mock("@tanstack/react-query", async (original) => ({
  ...(await original<typeof import("@tanstack/react-query")>()),
  useQueryClient: () => mock.queryClient,
  useQuery: () => ({
    isSuccess: !mock.optionsError,
    isError: mock.optionsError,
    isFetching: false,
    data: mock.optionsError ? undefined : { options: mock.options },
    refetch: mock.refetch,
  }),
}));
vi.mock("@/hooks/use-assistant-voice", () => ({
  useAssistantVoice: () => ({
    connected: mock.connected,
    starting: mock.starting,
    error: mock.error,
    speakerMuted: false,
    end: mock.end,
    start: mock.start,
    mute: mock.mute,
    muteSpeaker: mock.speaker,
    hold: mock.hold,
    stopTask: mock.stop,
    rejoin: mock.rejoin,
    conflictSession: mock.conflictSession,
    resumeAudio: vi.fn(),
    snapshot: {
      session: {
        state: "active",
        created_at: new Date(Date.now() - 65000).toISOString(),
        closed_at: null,
        muted: false,
        idle_warning: false,
      },
      captions: [
        {
          id: "u",
          speaker: "user",
          text: "What did I ask?",
          sealed: true,
          complete: true,
        },
        {
          id: "a",
          speaker: "assistant",
          text: "Your tasks are in this thread.",
          sealed: true,
          complete: true,
        },
      ],
      tasks: mock.tasks,
    },
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.queryClient = new QueryClient();
  mock.connected = true;
  mock.options = [];
  mock.optionsError = false;
  mock.starting = false;
  mock.error = null;
  mock.refetch.mockResolvedValue({});
  mock.conflictSession = null;
  mock.tasks = [];
  vi.spyOn(Date, "now").mockReturnValue(Date.parse("2026-10-04T00:00:00Z"));
});
afterEach(() => vi.restoreAllMocks());
it("minimizes and restores without ending the media session", () => {
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  expect(screen.getByLabelText("Your captions")).toHaveTextContent(
    "What did I ask?",
  );
  expect(screen.getByLabelText("Assistant captions")).toHaveTextContent(
    "Your tasks are in this thread.",
  );
  expect(screen.getByLabelText("Call duration")).toHaveTextContent("1:05");
  fireEvent.click(screen.getByRole("button", { name: "Minimize voice call" }));
  expect(screen.getByLabelText("Minimized voice call")).toBeInTheDocument();
  expect(screen.queryByLabelText("Your captions")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Mute microphone" }));
  expect(mock.mute).toHaveBeenCalledWith(true);
  fireEvent.click(screen.getByRole("button", { name: "Restore voice panel" }));
  expect(screen.getByLabelText("Your captions")).toBeInTheDocument();
  expect(mock.end).not.toHaveBeenCalled();
});

it("shows Rejoin and End call for a discovered active session", () => {
  mock.connected = false;
  mock.conflictSession = {
    id: "f985604b-1991-4aa2-969d-2325055ea813",
    resumable: true,
    control_revision: 0,
  };
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Rejoin" }));
  expect(mock.rejoin).toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "End call" }));
  expect(mock.end).toHaveBeenCalled();
});
it("speaker mute leaves captions and microphone controls available", () => {
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Mute speaker" }));
  expect(mock.speaker).toHaveBeenCalledWith(true);
  expect(mock.mute).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Assistant captions")).toHaveTextContent(
    "Your tasks are in this thread.",
  );
});

it("shows human task states and links settled results", () => {
  mock.tasks = [
    {
      id: "task-1",
      state: "completed",
      pending_acknowledgement_ids: [],
      title: "Check my calendar",
      result_message_id: "result-1",
    },
    {
      id: "task-2",
      state: "awaiting_confirmation",
      pending_acknowledgement_ids: ["card-1"],
      title: "Delete the old event",
      result_message_id: null,
    },
  ];
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  expect(screen.getByText(/Check my calendar · Done/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "result" })).toHaveAttribute(
    "href",
    "#message-result-1",
  );
  expect(
    screen.getByText(/Delete the old event · Needs your OK/),
  ).toBeInTheDocument();
});

it("starts with a catalog-added model and voice and keeps payer and tariff in Credit usage", async () => {
  mock.connected = false;
  mock.options = [
    {
      service_id: "service",
      connection_id: "connection",
      key_source: "own",
      model: "catalog-new-model",
      model_label: "Catalog model",
      default_model: true,
      available: true,
      billing_owner: "organization",
      unavailable_reason: null,
      voice: {
        protocol: "openai_live",
        models: [],
        voices: [{ id: "catalog-new-voice", label: "Catalog voice" }],
        usage_source: "provider_reported",
        billing_metrics: ["voice_seconds", "input_tokens"],
      },
      pricing: {
        metric: "input_tokens",
        credits_per_unit: "0.00002",
        sync_status: "synced",
        components: [
          {
            metric: "voice_seconds",
            credits_per_unit: "0.01",
            sync_status: "synced",
          },
        ],
      },
    },
  ];
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  expect(
    screen.getByRole("option", { name: "Catalog voice" }),
  ).toBeInTheDocument();
  expect(screen.getByText(/Paid by the organization/)).not.toBeVisible();
  fireEvent.click(screen.getByText("Credit usage"));
  expect(screen.getByText(/Paid by the organization/)).toHaveTextContent(
    "0.01 credits per second",
  );
  expect(screen.getByText(/Paid by the organization/)).toHaveTextContent(
    "0.00002",
  );
  expect(screen.getByText(/Charges are recorded/)).toHaveTextContent(
    "the organization's credits",
  );
  expect(
    screen.queryByText(/your account menu under Usage/),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({
        model: "catalog-new-model",
        voice: "catalog-new-voice",
      }),
      undefined,
      expect.any(Function),
    ),
  );
});

it("allows lane-less own-key voice and discloses metering-only pricing", async () => {
  mock.connected = false;
  mock.options = [
    {
      service_id: "service",
      connection_id: "connection",
      key_source: "own",
      model: "gpt-live-1",
      model_label: "GPT-Live",
      default_model: true,
      available: true,
      unavailable_reason: null,
      billing_owner: "acting_person",
      pricing: null,
      voice: {
        protocol: "openai_live",
        models: [],
        voices: [{ id: "marin", label: "Marin" }],
        usage_source: "provider_reported",
        billing_metrics: ["voice_seconds"],
      },
    },
  ];
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  fireEvent.click(screen.getByText("Credit usage"));
  expect(screen.getByText(/No NyxID voice charge/)).toHaveTextContent(
    "your provider's charges still apply",
  );
  expect(
    screen.queryByText(/Voice tariff unavailable/),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({
        key_source: "own",
        connection_id: "connection",
      }),
      undefined,
      expect.any(Function),
    ),
  );
});

it("labels Grok PTT, gates automatic mode, and discloses tokens when seconds are free", async () => {
  mock.connected = false;
  mock.options = [
    {
      service_id: "grok-service",
      connection_id: "connection",
      key_source: "own",
      model: "catalog-grok",
      model_label: "Grok",
      default_model: true,
      available: true,
      billing_owner: "acting_person",
      pricing: null,
      reported_token_pricing: {
        metric: "input_tokens",
        credits_per_unit: "0.0001",
        sync_status: "synced",
        components: [],
      },
      voice: {
        protocol: "xai_realtime",
        models: [],
        voices: [{ id: "eve", label: "Eve" }],
        usage_source: "server_measured",
        billing_metrics: ["voice_seconds", "input_tokens"],
      },
    },
  ];
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  expect(screen.getByText(/Grok private beta/)).toHaveTextContent(
    "headphones recommended",
  );
  expect(screen.getByRole("option", { name: /Automatic/ })).toBeDisabled();
  expect(screen.getByLabelText("Microphone mode")).toHaveValue("push_to_talk");
  fireEvent.click(screen.getByText("Credit usage"));
  expect(screen.getByText(/No NyxID duration charge/)).toHaveTextContent(
    "0.0001 credits per reported",
  );
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({
        model: "catalog-grok",
        input_mode: "push_to_talk",
      }),
      "xai_realtime",
      expect.any(Function),
    ),
  );
});

it("discloses configured Grok token prices even when catalog summary omits token metrics", () => {
  mock.connected = false;
  mock.options = [
    {
      service_id: "grok-service",
      connection_id: "connection",
      key_source: "own",
      model: "catalog-grok",
      model_label: "Grok",
      default_model: true,
      available: true,
      billing_owner: "acting_person",
      pricing: {
        metric: "voice_seconds",
        credits_per_unit: "0.02",
        sync_status: "synced",
        components: [
          {
            metric: "input_tokens",
            credits_per_unit: "0.0001",
            sync_status: "synced",
          },
        ],
      },
      voice: {
        protocol: "xai_realtime",
        models: [],
        voices: [{ id: "eve", label: "Eve" }],
        usage_source: "server_measured",
        billing_metrics: ["voice_seconds"],
      },
    },
  ];
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  expect(screen.getByText(/0.02 credits per second/)).toHaveTextContent(
    "0.0001",
  );
});

it("integrated active call is compact with accessible details and all media controls", () => {
  mock.options = [voiceOption];
  const close = vi.fn();
  render(
    <VoicePanel
      threadId="thread"
      panelId="panel"
      onClose={close}
      onViewChange={vi.fn()}
    />,
  );
  expect(
    screen.queryByRole("button", { name: "Minimize voice call" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Voice source")).not.toBeInTheDocument();
  expect(screen.queryByText(/credits per second/)).not.toBeInTheDocument();
  expect(screen.queryByText(/billed separately/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Voice settings" }));
  expect(screen.getByLabelText("Voice source")).toBeInTheDocument();
  fireEvent.click(screen.getByText("Credit usage"));
  expect(screen.getByText(/credits per second/)).toHaveTextContent(
    "Paid by you",
  );
  fireEvent.click(screen.getByRole("button", { name: "Voice settings" }));
  expect(screen.queryByText(/credits per second/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Mute speaker" }));
  expect(mock.speaker).toHaveBeenCalledWith(true);
  fireEvent.click(screen.getByRole("button", { name: "End call" }));
  expect(mock.end).toHaveBeenCalledOnce();
  expect(close).not.toHaveBeenCalled();
});

function preparation() {
  const value = {
    isClosed: false,
    close: vi.fn(() => {
      value.isClosed = true;
    }),
  };
  return value as unknown as PreparedMicrophone;
}
it("intentional waveform preparation starts OpenAI automatic once without inline prices", async () => {
  mock.connected = false;
  mock.options = [voiceOption];
  const prepared = preparation();
  const { rerender } = render(
    <VoicePanel
      threadId="thread"
      preparation={prepared}
      onClose={vi.fn()}
      onViewChange={vi.fn()}
    />,
  );
  expect(screen.queryByText(/credits per second/)).not.toBeInTheDocument();
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({ input_mode: "automatic" }),
      undefined,
      expect.any(Function),
      prepared,
    ),
  );
  rerender(
    <VoicePanel
      threadId="thread"
      preparation={prepared}
      onClose={vi.fn()}
      onViewChange={vi.fn()}
    />,
  );
  expect(mock.start).toHaveBeenCalledOnce();
});
it("saved unavailable source never falls back; chosen settings survive the next waveform preparation", async () => {
  mock.connected = false;
  mock.options = [voiceOption];
  const first = preparation();
  const saved = {
    ...voicePreferences,
    service_id: "missing",
    input_mode: "push_to_talk" as const,
  };
  const props = {
    threadId: "thread",
    savedPreferences: saved,
    onClose: vi.fn(),
    onViewChange: vi.fn(),
  };
  const { rerender } = render(<VoicePanel {...props} preparation={first} />);
  await waitFor(() => expect(first.close).toHaveBeenCalledOnce());
  expect(mock.start).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Voice settings" }));
  fireEvent.change(screen.getByLabelText("Voice source"), {
    target: {
      value: `${voiceOption.service_id}:platform:${voiceOption.model}`,
    },
  });
  expect(screen.getByLabelText("Microphone mode")).toHaveValue("push_to_talk");
  fireEvent.change(screen.getByLabelText("Microphone mode"), {
    target: { value: "automatic" },
  });
  const next = preparation();
  rerender(<VoicePanel {...props} preparation={next} />);
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({
        service_id: voiceOption.service_id,
        input_mode: "automatic",
      }),
      undefined,
      expect.any(Function),
      next,
    ),
  );
});
it("configuration does not capture/start; End retains next-call input choice in the same panel", async () => {
  mock.options = [voiceOption];
  const close = vi.fn();
  const props = { threadId: "thread", onClose: close, onViewChange: vi.fn() };
  const { rerender } = render(<VoicePanel {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "End call" }));
  expect(close).not.toHaveBeenCalled();
  mock.connected = false;
  rerender(<VoicePanel {...props} settingsRequest={1} />);
  fireEvent.change(screen.getByLabelText("Microphone mode"), {
    target: { value: "push_to_talk" },
  });
  expect(mock.start).not.toHaveBeenCalled();
  const next = preparation();
  rerender(<VoicePanel {...props} settingsRequest={1} preparation={next} />);
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({ input_mode: "push_to_talk" }),
      undefined,
      expect.any(Function),
      next,
    ),
  );
});

it("starts with unchanged saved preferences without waiting for a redundant settings write", async () => {
  useAuthStore.setState({
    user: { id: "same-settings-owner" } as NonNullable<
      ReturnType<typeof useAuthStore.getState>["user"]
    >,
  });
  mock.connected = false;
  mock.options = [voiceOption];
  const saved = { ...voicePreferences, voice: "marin" };
  const provider = vi.fn();
  mock.start.mockImplementationOnce(
    async (_prefs, _protocol, before: () => Promise<void>) => {
      await before();
      provider();
    },
  );
  render(
    <VoicePanel threadId="thread" savedPreferences={saved} onClose={vi.fn()} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  await waitFor(() => expect(provider).toHaveBeenCalledOnce());
  expect(mock.save).not.toHaveBeenCalled();
});

it("the unchanged-preferences path still refuses a switched account", async () => {
  useAuthStore.setState({
    user: { id: "original-owner" } as NonNullable<
      ReturnType<typeof useAuthStore.getState>["user"]
    >,
  });
  mock.connected = false;
  mock.options = [voiceOption];
  let authorize!: () => Promise<void>;
  mock.start.mockImplementationOnce(
    async (_prefs, _protocol, before: () => Promise<void>) => {
      authorize = before;
    },
  );
  render(
    <VoicePanel
      threadId="thread"
      savedPreferences={{ ...voicePreferences, voice: "marin" }}
      onClose={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  act(() =>
    useAuthStore.setState({
      user: { id: "other-owner" } as NonNullable<
        ReturnType<typeof useAuthStore.getState>["user"]
      >,
    }),
  );
  await expect(authorize()).rejects.toThrow("Your account changed");
  expect(mock.save).not.toHaveBeenCalled();
});

it("confirmed preference save updates the actor cache so Close/reopen retains source and input mode", async () => {
  const actor = { id: "preference-owner" } as NonNullable<
    ReturnType<typeof useAuthStore.getState>["user"]
  >;
  useAuthStore.setState({ user: actor });
  mock.connected = false;
  const chosen = {
    ...voiceOption,
    service_id: "22222222-2222-4222-8222-222222222222",
    model_label: "Chosen source",
  };
  mock.options = [voiceOption, chosen];
  const key = nyxBotQueryKeys.settings(actor.id);
  mock.queryClient!.setQueryData(key, { voice: voicePreferences });
  let confirm!: (settings: { voice: VoicePreferences }) => void;
  mock.save.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        confirm = resolve;
      }),
  );
  mock.start.mockImplementationOnce(
    async (_prefs, _protocol, before: () => Promise<void>) => {
      await before();
    },
  );
  const cached = () =>
    mock.queryClient!.getQueryData<{ voice: VoicePreferences }>(key)!.voice;
  const view = render(
    <VoicePanel
      threadId="thread"
      savedPreferences={cached()}
      onClose={vi.fn()}
    />,
  );
  fireEvent.change(screen.getByLabelText("Voice source"), {
    target: { value: `${chosen.service_id}:platform:${chosen.model}` },
  });
  fireEvent.change(screen.getByLabelText("Microphone mode"), {
    target: { value: "automatic" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  await waitFor(() =>
    expect(mock.save).toHaveBeenCalledWith({
      voice: expect.objectContaining({
        service_id: chosen.service_id,
        input_mode: "automatic",
      }),
    }),
  );
  expect(cached().input_mode).toBe("push_to_talk"); // No optimistic replacement.
  const saved = {
    ...voicePreferences,
    service_id: chosen.service_id,
    voice: "marin",
    input_mode: "automatic" as const,
  };
  confirm({ voice: saved });
  await waitFor(() => expect(cached()).toEqual(saved));
  view.unmount();
  render(
    <VoicePanel
      threadId="thread"
      savedPreferences={cached()}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByLabelText("Microphone mode")).toHaveValue("automatic");
  expect(screen.getByLabelText("Voice source")).toHaveValue(
    `${chosen.service_id}:platform:${chosen.model}`,
  );
});

it("an account change during confirmed save cannot replace either actor cache or authorize provider start", async () => {
  const actor = { id: "saving-owner" } as NonNullable<
    ReturnType<typeof useAuthStore.getState>["user"]
  >;
  useAuthStore.setState({ user: actor });
  mock.connected = false;
  mock.options = [voiceOption];
  const originalKey = nyxBotQueryKeys.settings(actor.id);
  const otherKey = nyxBotQueryKeys.settings("new-owner");
  mock.queryClient!.setQueryData(originalKey, { voice: voicePreferences });
  mock.queryClient!.setQueryData(otherKey, { voice: null });
  let authorize!: () => Promise<void>;
  mock.start.mockImplementationOnce(
    async (_prefs, _protocol, before: () => Promise<void>) => {
      authorize = before;
    },
  );
  let confirm!: (settings: { voice: VoicePreferences }) => void;
  mock.save.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        confirm = resolve;
      }),
  );
  render(
    <VoicePanel
      threadId="thread"
      savedPreferences={voicePreferences}
      onClose={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  await waitFor(() => expect(authorize).toBeTypeOf("function"));
  const saving = authorize();
  act(() => useAuthStore.setState({ user: { ...actor, id: "new-owner" } }));
  confirm({ voice: { ...voicePreferences, input_mode: "automatic" } });
  await expect(saving).rejects.toThrow("Voice preferences could not be saved");
  expect(mock.queryClient!.getQueryData(originalKey)).toEqual({
    voice: voicePreferences,
  });
  expect(mock.queryClient!.getQueryData(otherKey)).toEqual({ voice: null });
});

it("compact startup error offers only an intentional retry of the exact selected voice source", async () => {
  mock.connected = false;
  mock.error =
    "Insufficient voice credits. Check your available credits, then retry voice.";
  mock.options = [voiceOption];
  const props = {
    threadId: "thread",
    savedPreferences: voicePreferences,
    onClose: vi.fn(),
    onViewChange: vi.fn(),
  };
  const view = render(<VoicePanel {...props} />);
  expect(mock.start).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Check your available credits",
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Retry voice" }),
  );
  expect(mock.start).toHaveBeenCalledWith(
    expect.objectContaining({
      service_id: voicePreferences.service_id,
      model: voicePreferences.model,
      input_mode: voicePreferences.input_mode,
    }),
    undefined,
    expect.any(Function),
  );
  // No failed prepared capture is reused: start() acquires new media from this gesture.
  mock.starting = true;
  view.rerender(<VoicePanel {...props} />);
  expect(
    screen.queryByRole("button", { name: "Retry voice" }),
  ).not.toBeInTheDocument();
  mock.starting = false;
  mock.conflictSession = {
    id: "active-call",
    resumable: true,
    control_revision: 0,
  };
  view.rerender(<VoicePanel {...props} />);
  expect(
    screen.queryByRole("button", { name: "Retry voice" }),
  ).not.toBeInTheDocument();
  mock.conflictSession = null;
  view.rerender(<VoicePanel {...props} preferencesReady={false} />);
  expect(
    screen.queryByRole("button", { name: "Retry voice" }),
  ).not.toBeInTheDocument();
  view.rerender(
    <VoicePanel
      {...props}
      savedPreferences={{ ...voicePreferences, service_id: "unavailable" }}
    />,
  );
  expect(
    screen.queryByRole("button", { name: "Retry voice" }),
  ).not.toBeInTheDocument();
});

it("failed source discovery can be retried without capture or paid-source fallback", async () => {
  mock.connected = false;
  mock.optionsError = true;
  const first = preparation();
  const props = {
    threadId: "thread",
    savedPreferences: voicePreferences,
    onClose: vi.fn(),
    onViewChange: vi.fn(),
  };
  const view = render(<VoicePanel {...props} preparation={first} />);
  await waitFor(() => expect(first.close).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "Retry voice loading" }));
  await waitFor(() => expect(mock.refetch).toHaveBeenCalledOnce());
  expect(mock.start).not.toHaveBeenCalled();
  mock.optionsError = false;
  mock.options = [voiceOption];
  view.rerender(<VoicePanel {...props} preparation={first} />);
  expect(mock.start).not.toHaveBeenCalled();
  const next = preparation();
  view.rerender(<VoicePanel {...props} preparation={next} />);
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({
        service_id: voicePreferences.service_id,
        input_mode: voicePreferences.input_mode,
      }),
      undefined,
      expect.any(Function),
      next,
    ),
  );
});

it("failed saved preferences can be retried and hydrate before editing or the next start", async () => {
  mock.connected = false;
  mock.options = [voiceOption];
  const retry = vi.fn().mockResolvedValue({});
  const first = preparation();
  const props = {
    threadId: "thread",
    onClose: vi.fn(),
    onViewChange: vi.fn(),
    settingsRequest: 1,
    onRetryPreferences: retry,
  };
  const view = render(
    <VoicePanel {...props} preferencesError preparation={first} />,
  );
  await waitFor(() => expect(first.close).toHaveBeenCalledOnce());
  expect(screen.getByLabelText("Microphone mode")).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Retry voice loading" }));
  await waitFor(() => expect(retry).toHaveBeenCalledOnce());
  view.rerender(<VoicePanel {...props} preferencesReady={false} />);
  expect(screen.getByLabelText("Microphone mode")).toBeDisabled();
  view.rerender(<VoicePanel {...props} savedPreferences={voicePreferences} />);
  expect(screen.getByLabelText("Microphone mode")).toBeEnabled();
  expect(screen.getByLabelText("Microphone mode")).toHaveValue(
    voicePreferences.input_mode,
  );
  fireEvent.change(screen.getByLabelText("Microphone mode"), {
    target: { value: "automatic" },
  });
  // A later query refresh cannot overwrite the explicit next-call edit.
  view.rerender(
    <VoicePanel {...props} savedPreferences={{ ...voicePreferences }} />,
  );
  expect(screen.getByLabelText("Microphone mode")).toHaveValue("automatic");
  expect(mock.start).not.toHaveBeenCalled();
  const next = preparation();
  view.rerender(
    <VoicePanel
      {...props}
      savedPreferences={voicePreferences}
      preparation={next}
    />,
  );
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({ input_mode: "automatic" }),
      undefined,
      expect.any(Function),
      next,
    ),
  );
});
