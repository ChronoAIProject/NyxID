import { act, fireEvent, render, screen } from "@testing-library/react";
import { lazy } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ScenarioEngine } from "@/lib/assistant/scenario-engine";
import { compiledScenarios } from "@/lib/assistant/scenarios.config";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";
import { MockScenariosAction } from "./mock-scenarios-action";

vi.mock("@/components/assistant/assistant-wire-log-panel", () => ({
  AssistantWireLogAction: () => null,
}));

import { AssistantHeaderActions } from "@/pages/assistant";

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

    render(
      <div>
        <p>Assistant shell</p>
        <AssistantHeaderActions scenarioAction={DeferredAction} />
        <AssistantHeaderActions scenarioAction={DeferredAction} />
      </div>,
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
    render(<AssistantHeaderActions scenarioAction={null} />);

    expect(
      screen.queryByRole("button", { name: "Mock scenarios" }),
    ).not.toBeInTheDocument();
  });
});
