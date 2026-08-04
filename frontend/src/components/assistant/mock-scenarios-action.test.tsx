import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { lazy, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ScenarioEngine } from "@/lib/assistant/scenario-engine";
import { compiledScenarios } from "@/lib/assistant/scenarios.config";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";
import { useAuthStore } from "@/stores/auth-store";
import { MockScenariosAction } from "./mock-scenarios-action";

vi.mock("@/components/assistant/assistant-wire-log-panel", () => ({
  AssistantWireLogAction: () => null,
}));

import { AssistantHeaderActions } from "@/pages/assistant";

/** Render the header with the platform flag resolved to `featureEnabled`. */
function renderHeader(children: ReactNode, featureEnabled: boolean) {
  useAuthStore.setState({
    user: {
      capabilities: {
        enabled_features: featureEnabled
          ? [FEATURE_FLAG.ASSISTANT_MOCK_SCENARIOS]
          : [],
      },
    } as never,
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>,
  );
}

function renderAction() {
  return render(
    <TooltipProvider>
      <MockScenariosAction />
    </TooltipProvider>,
  );
}

function openPopover(): void {
  fireEvent.click(screen.getByRole("button", { name: "Mock scenarios" }));
}

describe("MockScenariosAction", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantMockScenariosStore.getState().reset();
    useAssistantMockScenariosStore.setState({ engineState: "ready" });
  });

  afterEach(() => {
    useAuthStore.setState({ user: null });
  });

  it("shows loading and engine failure states inline (P11)", () => {
    useAssistantMockScenariosStore.setState({ engineState: "loading" });
    renderAction();
    openPopover();

    expect(screen.getByRole("status")).toHaveTextContent("Loading...");
    expect(
      screen.getByRole("switch", { name: "Enable mock scenarios" }),
    ).toBeDisabled();

    act(() => {
      useAssistantMockScenariosStore.setState({ engineState: "error" });
    });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Scenario engine failed to load.",
    );
  });

  it("renders matched and unmatched activity with relative time (P11)", () => {
    const now = Date.now();
    useAssistantMockScenariosStore.setState({
      lastActivity: {
        scenarioId: "github-issues",
        matched: true,
        at: now,
      },
    });
    renderAction();
    openPopover();

    expect(
      screen.getByTestId("scenario-activity-github-issues"),
    ).toHaveTextContent("Matched just now");

    act(() => {
      useAssistantMockScenariosStore.setState({
        lastActivity: { scenarioId: null, matched: false, at: now },
      });
    });
    expect(screen.getByTestId("unmatched-scenario-activity")).toHaveTextContent(
      "No scenario matched - just now; message passed through.",
    );
  });

  it("disables a scenario in the store and removes it from interception matching (P11)", () => {
    renderAction();
    openPopover();

    fireEvent.click(
      screen.getByRole("switch", {
        name: "Enable connect-github scenario",
      }),
    );

    const disabledScenarioIds =
      useAssistantMockScenariosStore.getState().disabledScenarioIds;
    expect(disabledScenarioIds).toEqual(["connect-github"]);

    const engine = new ScenarioEngine(compiledScenarios, {
      isConnected: () => false,
      connect: () => undefined,
      disconnect: () => undefined,
    });
    expect(
      engine.match("connect to my github", disabledScenarioIds),
    ).toBeNull();
  });

  it("removes connected-world chips, resets the world, and renders its empty state (P11)", () => {
    useAssistantMockScenariosStore.getState().connectService("api-github");
    useAssistantMockScenariosStore.getState().connectService("api-openai");
    renderAction();
    openPopover();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Remove api-github from mock world",
      }),
    );
    expect(useAssistantMockScenariosStore.getState().world.connected).toEqual([
      "api-openai",
    ]);

    fireEvent.click(screen.getByRole("button", { name: "Reset world" }));
    expect(useAssistantMockScenariosStore.getState().world.connected).toEqual(
      [],
    );
    expect(screen.getByText(/Nothing connected -/)).toBeInTheDocument();
    expect(screen.getByText("need")).toBeInTheDocument();
  });

  it("discloses pass-through behavior, real account journeys, and persistence semantics (P9, P11)", () => {
    useAssistantMockScenariosStore.getState().setEnabled(true);
    renderAction();

    expect(screen.getByTestId("mock-scenarios-active-dot")).toBeInTheDocument();
    openPopover();
    expect(
      screen.getByText(
        /Intercepts matching chat messages with scripted flows\. Session-only;/,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Action cards open real connection journeys and can create real keys on your account.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Settings use last-write-wins across tabs."),
    ).toBeInTheDocument();
  });

  it("keeps both assistant header mounts visible while their lazy action is deferred (P8, P11)", async () => {
    let resolveAction: (module: {
      default: () => React.JSX.Element;
    }) => void = () => undefined;
    const DeferredAction = lazy(
      () =>
        new Promise<{ default: () => React.JSX.Element }>((resolve) => {
          resolveAction = resolve;
        }),
    );

    renderHeader(
      <div>
        <p>Assistant shell</p>
        <AssistantHeaderActions scenarioAction={DeferredAction} />
        <AssistantHeaderActions scenarioAction={DeferredAction} />
      </div>,
      true,
    );

    expect(screen.getByText("Assistant shell")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Deferred mock scenarios" }),
    ).not.toBeInTheDocument();

    await act(async () => {
      resolveAction({
        default: () => (
          <button type="button" aria-label="Deferred mock scenarios" />
        ),
      });
    });

    expect(
      await screen.findAllByRole("button", {
        name: "Deferred mock scenarios",
      }),
    ).toHaveLength(2);
  });

  it("omits the developer action when the module gate supplies null (F12, P11)", () => {
    renderHeader(<AssistantHeaderActions scenarioAction={null} />, true);

    expect(
      screen.queryByRole("button", { name: "Mock scenarios" }),
    ).not.toBeInTheDocument();
  });

  it("omits the developer action when the platform flag is off", () => {
    const Action = () => <button type="button" aria-label="Mock scenarios" />;

    renderHeader(<AssistantHeaderActions scenarioAction={Action} />, false);

    // Fail-closed: an unflagged user gets no control even though the component
    // was handed to the header, and the lazy chunk is never requested.
    expect(
      screen.queryByRole("button", { name: "Mock scenarios" }),
    ).not.toBeInTheDocument();
  });

  it("renders the developer action once the platform flag resolves on", () => {
    const Action = () => <button type="button" aria-label="Mock scenarios" />;

    renderHeader(<AssistantHeaderActions scenarioAction={Action} />, true);

    expect(
      screen.getByRole("button", { name: "Mock scenarios" }),
    ).toBeInTheDocument();
  });
});
