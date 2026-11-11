import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { VoicePanel } from "./voice-panel";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { nyxBotQueryKeys, useNyxBotSettings } from "@/hooks/use-nyxbot-agents";
import { nyxAgentSettingsSchema } from "@/schemas/assistant-nyxagent";

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: Object.assign(
    (select: (state: { user: { id: string } }) => unknown) =>
      select({ user: { id: "owner" } }),
    { getState: () => ({ user: { id: "owner" } }) },
  ),
}));
vi.mock("@/hooks/use-assistant-voice", () => ({
  useAssistantVoice: () => ({
    connected: false,
    starting: false,
    start: vi.fn(),
    end: vi.fn(),
  }),
}));
afterEach(() => vi.restoreAllMocks());

it("restores a changed account voice in another conversation and skips an unchanged save", async () => {
  const voice = {
    service_id: "11111111-1111-4111-8111-111111111111",
    connection_id: "22222222-2222-4222-8222-222222222222",
    key_source: "own" as const,
    model: "gpt-live-1",
    voice: "marin",
    input_mode: "push_to_talk" as const,
    language: null,
    notify_on_completion: false,
  };
  const settings = nyxAgentSettingsSchema.parse({
    voice,
    skip_destructive_confirmation: false,
    max_live_subagents: 8,
    max_concurrent_subagent_turns: 3,
    max_live_subagents_limit: 32,
    max_concurrent_subagent_turns_limit: 8,
  });
  const save = vi
    .spyOn(nyxBotApi, "updateSettings")
    .mockImplementation(async (update) => ({ ...settings, ...update }));
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(nyxBotQueryKeys.settings("owner"), settings);
  for (const thread of ["one", "two"])
    client.setQueryData(["assistant-voice-options", "owner", thread], {
      options: [
        {
          ...voice,
          available: true,
          default_model: true,
          model_label: "GPT Live",
          billing_owner: "acting_person",
          pricing: null,
          voice: {
            protocol: "openai_live",
            voices: [
              { id: "marin", label: "Marin" },
              { id: "cedar", label: "Cedar" },
            ],
            billing_metrics: ["voice_seconds"],
          },
        },
      ],
    });
  function Settings({ thread }: { thread: string }) {
    const saved = useNyxBotSettings();
    return (
      <VoicePanel
        key={thread}
        threadId={thread}
        settingsOnly
        savedPreferences={saved.data?.voice}
        onClose={() => {}}
      />
    );
  }
  const view = render(
    <QueryClientProvider client={client}>
      <Settings thread="one" />
    </QueryClientProvider>,
  );
  fireEvent.change(screen.getByLabelText("Voice"), {
    target: { value: "cedar" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save voice settings" }));
  await waitFor(() =>
    expect(
      client.getQueryData(nyxBotQueryKeys.settings("owner")),
    ).toMatchObject({ voice: { voice: "cedar" } }),
  );
  view.rerender(
    <QueryClientProvider client={client}>
      <Settings thread="two" />
    </QueryClientProvider>,
  );
  expect(screen.getByLabelText("Voice")).toHaveValue("cedar");
  fireEvent.click(screen.getByRole("button", { name: "Save voice settings" }));
  expect(save).toHaveBeenCalledOnce();
  view.unmount();
  client.clear();
});
