import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ActionReport } from "@/schemas/assistant-actions";
import type {
  ActionCardContentBlock,
  ConversationHistory,
} from "@/types/assistant";

const TEST_NOW = Date.parse("2026-08-04T02:00:00.000Z");

function createHarness() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { queryClient, Wrapper };
}

function findActionCard(
  history: ConversationHistory | undefined,
): ActionCardContentBlock | undefined {
  for (const message of history?.messages ?? []) {
    const card = message.blocks.find((block) => block.type === "action_card");
    if (card?.type === "action_card") return card;
  }
  return undefined;
}

beforeEach(() => {
  localStorage.clear();
  vi.resetModules();
  vi.useFakeTimers();
  vi.setSystemTime(TEST_NOW);
  vi.stubEnv("MODE", "development");
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("assistant mock-scenario hook integration", () => {
  it("preserves a parked overlay across refetch and renders its verified continuation (#1304, P1)", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/keys/service-1")) {
        return new Response(
          JSON.stringify({
            id: "service-1",
            catalog_service_slug: "api-github",
          }),
          {
            status: 200,
            headers: { "Content-Type": "application/json" },
          },
        );
      }
      return new Response(
        JSON.stringify({
          error: "not_found",
          error_code: 0,
          message: "No materialized live transcript.",
        }),
        {
          status: 404,
          headers: { "Content-Type": "application/json" },
        },
      );
    });
    vi.stubGlobal("fetch", fetchMock);

    const transportModule = await import("@/lib/assistant/transport");
    await import("@/lib/assistant/scenario-intercept-transport");
    await Promise.resolve();
    const storeModule = await import("@/stores/assistant-mock-scenarios-store");
    const hooks = await import("./use-assistant");

    expect(transportModule.assistantTransport).toBeInstanceOf(
      transportModule.DelegatingAssistantTransport,
    );
    // Importing the transport installs nothing on its own; the platform flag
    // is what arms the singleton, exactly as `MockScenariosGate` does it.
    expect(
      storeModule.useAssistantMockScenariosStore.getState().engineState,
    ).toBe("idle");
    await transportModule.applyAssistantScenarioFeature(true);
    expect(
      storeModule.useAssistantMockScenariosStore.getState().engineState,
    ).toBe("ready");
    storeModule.useAssistantMockScenariosStore.setState({
      featureEnabled: true,
      enabled: true,
      disabledScenarioIds: [],
      world: { connected: [] },
      engineState: "ready",
      lastActivity: null,
    });

    const { queryClient, Wrapper } = createHarness();
    let conversationId: string | undefined;
    const hook = renderHook(
      () => ({
        history: hooks.useConversation(conversationId),
        send: hooks.useSendMessage(conversationId),
        turn: hooks.useAssistantTurn(conversationId),
        actions: hooks.useActionCardActions(conversationId),
      }),
      { wrapper: Wrapper },
    );

    await act(async () => {
      const sent = await hook.result.current.send.mutateAsync(
        "connect to my github",
      );
      conversationId = sent.conversationId;
      await vi.runAllTimersAsync();
    });
    hook.rerender();
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    if (!conversationId) throw new Error("Expected a mock conversation id.");
    const historyKey = hooks.assistantKeys.history(conversationId);
    const parkedHistory =
      queryClient.getQueryData<ConversationHistory>(historyKey);
    const parkedCard = findActionCard(parkedHistory);
    expect(conversationId).toMatch(/^workflow-pending-/);
    expect(hook.result.current.turn.data?.status).toBe("blocked");
    expect(parkedCard).toMatchObject({
      action: "service.connect",
      status: "pending",
    });
    expect(
      parkedHistory?.messages.some((message) => message.role === "user"),
    ).toBe(true);

    await act(async () => {
      await hook.result.current.history.refetch();
    });
    const refetchedCard = findActionCard(
      queryClient.getQueryData<ConversationHistory>(historyKey),
    );
    expect(refetchedCard).toMatchObject({
      block_id: parkedCard?.block_id,
      action_request_id: parkedCard?.action_request_id,
      origin_turn_id: parkedCard?.origin_turn_id,
      status: "pending",
    });

    if (!parkedCard) throw new Error("Expected a parked action card.");
    const report: ActionReport = {
      actionRequestId: parkedCard.action_request_id,
      originTurnId: parkedCard.origin_turn_id,
      disposition: "completed",
      resource: { userService: { userServiceId: "service-1" } },
    };
    await act(async () => {
      await hook.result.current.actions.continueAction(report);
      await Promise.resolve();
      await vi.runAllTimersAsync();
    });

    const completedHistory =
      queryClient.getQueryData<ConversationHistory>(historyKey);
    const renderedText = (completedHistory?.messages ?? [])
      .flatMap((message) => message.blocks)
      .filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("\n");
    expect(renderedText).toContain("GitHub connected");
    expect(findActionCard(completedHistory)?.status).toBe("completed");
    expect(
      storeModule.useAssistantMockScenariosStore.getState().world.connected,
    ).toContain("api-github");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/keys/service-1",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    hook.unmount();
    queryClient.clear();
  });
});
