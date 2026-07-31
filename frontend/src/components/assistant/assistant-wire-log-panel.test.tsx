import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import type { User } from "@/types/api";
import {
  AssistantWireLogAction,
  AssistantWireLogPanel,
} from "./assistant-wire-log-panel";

const admin: User = {
  id: "admin-1",
  email: "admin@example.com",
  display_name: "Admin",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: true,
  role: "admin",
  is_active: true,
  created_at: "2026-07-31T00:00:00.000Z",
};

function renderWithTooltips(node: React.ReactNode) {
  return render(<TooltipProvider>{node}</TooltipProvider>);
}

describe("AssistantWireLogPanel", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantWireLogStore.setState({
      captureEnabled: true,
      showResponses: true,
      entries: [],
      totalBytes: 0,
      captureBytes: 0,
    });
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
  });

  it("renders, expands, and clears captured entries", () => {
    useAssistantWireLogStore.getState().recordExchange(
      [
        {
          degraded: false,
          method: "POST",
          path: "api/chat",
          commandType: "text",
          body: { type: "text", prompt: "inspect this payload" },
          headers: { accept: "text/event-stream" },
          identity: {
            mode: "jwt",
            forward_access_token: false,
            inject_delegation_token: true,
            bridge_minted: false,
          },
          truncated: false,
          response: null,
          upstreamOutcome: "no_response",
        },
      ],
      "sse",
      200,
    );
    renderWithTooltips(<AssistantWireLogPanel />);

    fireEvent.click(screen.getByRole("button", { name: "Aevatar wire log" }));
    expect(screen.getByText("/api/chat")).toBeInTheDocument();
    fireEvent.click(screen.getByText("/api/chat").closest("button")!);
    expect(screen.getByText(/inspect this payload/)).toBeInTheDocument();
    expect(screen.getByText(/"path": "\/api\/chat"/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /clear/i }));
    expect(screen.getByText("No captured requests")).toBeInTheDocument();
    expect(useAssistantWireLogStore.getState().entries).toHaveLength(0);
  });

  it("hides the icon for a non-admin user", () => {
    renderWithTooltips(
      <AssistantWireLogAction
        user={{ ...admin, is_admin: false, role: "user" }}
      />,
    );

    expect(
      screen.queryByRole("button", { name: "Aevatar wire log" }),
    ).not.toBeInTheDocument();
  });
});
