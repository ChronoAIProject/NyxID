import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { VoicePanel } from "./voice-panel";
vi.mock("@/lib/api-client", () => ({
  api: { put: vi.fn().mockResolvedValue({}) },
}));
const mock = vi.hoisted(() => ({
  connected: true,
  options: [] as unknown[],
  end: vi.fn(),
  start: vi.fn(),
  mute: vi.fn(),
  speaker: vi.fn(),
  hold: vi.fn(),
  stop: vi.fn(),
}));
vi.mock("@tanstack/react-query", () => ({
  useQuery: () => ({ isSuccess: true, data: { options: mock.options } }),
}));
vi.mock("@/hooks/use-assistant-voice", () => ({
  useAssistantVoice: () => ({
    connected: mock.connected,
    starting: false,
    error: null,
    speakerMuted: false,
    end: mock.end,
    start: mock.start,
    mute: mock.mute,
    muteSpeaker: mock.speaker,
    hold: mock.hold,
    stopTask: mock.stop,
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
      tasks: [],
    },
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.connected = true;
  mock.options = [];
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
it("speaker mute leaves captions and microphone controls available", () => {
  render(<VoicePanel threadId="thread" onClose={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Mute speaker" }));
  expect(mock.speaker).toHaveBeenCalledWith(true);
  expect(mock.mute).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Assistant captions")).toHaveTextContent(
    "Your tasks are in this thread.",
  );
});

it("starts with a catalog-added model and voice and discloses the payer and tariff", async () => {
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
  expect(screen.getByText(/Paid by the organization/)).toHaveTextContent(
    "0.01 credits per second",
  );
  expect(screen.getByText(/Paid by the organization/)).toHaveTextContent(
    "0.00002",
  );
  fireEvent.click(screen.getByRole("button", { name: "Start voice" }));
  await waitFor(() =>
    expect(mock.start).toHaveBeenCalledWith(
      expect.objectContaining({
        model: "catalog-new-model",
        voice: "catalog-new-voice",
      }),
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
