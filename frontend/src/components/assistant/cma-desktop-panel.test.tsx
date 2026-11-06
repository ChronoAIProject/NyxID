import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { CmaDesktopPanel } from "./cma-desktop-panel";
const mocks = vi.hoisted(() => ({ desktop: vi.fn() }));
vi.mock("@/hooks/use-nyxbot-cma-agents", () => ({
  useCmaDesktop: mocks.desktop,
}));
beforeEach(() => vi.clearAllMocks());
it.each([
  ["desktop_busy", /another viewer/i],
  ["agent_stopped", /stopped/i],
  ["agent_paused", /Agent is paused/i],
  ["agent_state_unknown", /state could not be confirmed/i],
  ["machine_not_included", /desktop capability/i],
  ["desktop_not_enabled", /desktop capability/i],
  ["sandbox_retired", /environment is retired/i],
  ["sandbox_retiring", /environment is being retired/i],
  ["authorization_expired", /authorization/i],
])("explains %s without substituting another Agent", (reason, message) => {
  mocks.desktop.mockReturnValue({
    data: {
      available: false,
      reason,
      shared: true,
      width: null,
      height: null,
      websocket_path: null,
    },
    refetch: vi.fn(),
  });
  render(<CmaDesktopPanel agentId="local-agent" />);
  expect(screen.getByText(message)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /create|revive|switch agent/i }),
  ).not.toBeInTheDocument();
});
it("refuses a descriptor that sends the browser to an external websocket", () => {
  mocks.desktop.mockReturnValue({
    data: {
      available: true,
      reason: null,
      shared: true,
      width: 1280,
      height: 720,
      websocket_path: "wss://other.example/socket",
    },
    refetch: vi.fn(),
  });
  render(<CmaDesktopPanel agentId="local-agent" />);
  expect(screen.getByText(/could not be verified/i)).toBeInTheDocument();
});
