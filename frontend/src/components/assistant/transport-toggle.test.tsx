import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantTransportStore } from "@/stores/assistant-transport-store";

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  invalidateQueries: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: mocks.invalidateQueries }),
}));

import { TransportToggle } from "./transport-toggle";

function renderToggle(turnActive = false) {
  return render(
    <TooltipProvider>
      <TransportToggle turnActive={turnActive} />
    </TooltipProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useAssistantTransportStore.setState({ mode: "chat" });
});

afterEach(() => {
  useAssistantTransportStore.setState({ mode: "chat" });
});

describe("TransportToggle", () => {
  it("renders all options with the stored mode checked", () => {
    renderToggle();
    expect(screen.getByRole("radiogroup", { name: "Chat API" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Chat" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByRole("radio", { name: "Completions" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.getByRole("radio", { name: "Workflow" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("reflects a stored completions mode", () => {
    useAssistantTransportStore.setState({ mode: "completions" });
    renderToggle();
    expect(screen.getByRole("radio", { name: "Completions" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("switches to the workflow mode", async () => {
    const user = userEvent.setup();
    renderToggle();

    await user.click(screen.getByRole("radio", { name: "Workflow" }));

    expect(useAssistantTransportStore.getState().mode).toBe("workflow");
    expect(mocks.invalidateQueries).toHaveBeenCalledWith({
      queryKey: ["assistant"],
    });
    expect(mocks.navigate).toHaveBeenCalledWith({
      to: "/assistant",
      search: {},
    });
  });

  it("switches mode, invalidates assistant queries, and drops the ?c selection", async () => {
    const user = userEvent.setup();
    renderToggle();

    await user.click(screen.getByRole("radio", { name: "Completions" }));

    expect(useAssistantTransportStore.getState().mode).toBe("completions");
    expect(mocks.invalidateQueries).toHaveBeenCalledWith({
      queryKey: ["assistant"],
    });
    // The two transports own disjoint conversation ids, so any `?c` pointing
    // at the other mode's world must not survive the switch.
    expect(mocks.navigate).toHaveBeenCalledWith({
      to: "/assistant",
      search: {},
    });
    expect(screen.getByRole("radio", { name: "Completions" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("does nothing when the already-active option is clicked", async () => {
    const user = userEvent.setup();
    renderToggle();

    await user.click(screen.getByRole("radio", { name: "Chat" }));

    expect(mocks.invalidateQueries).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it("is inert while a turn is streaming", async () => {
    const user = userEvent.setup();
    renderToggle(true);

    const completions = screen.getByRole("radio", { name: "Completions" });
    expect(completions).toHaveAttribute("aria-disabled", "true");
    await user.click(completions);

    expect(useAssistantTransportStore.getState().mode).toBe("chat");
    expect(mocks.invalidateQueries).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
});
