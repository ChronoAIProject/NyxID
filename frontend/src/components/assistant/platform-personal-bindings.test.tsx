import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PlatformPersonalBindings } from "./platform-personal-bindings";
import { apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";

vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  apiClient: vi.fn(),
}));
const binding = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
let client: QueryClient;
beforeEach(() => {
  vi.mocked(apiClient).mockReset();
  useAuthStore.setState({
    user: { id: "owner", display_name: "Owner" } as never,
    isLoading: false,
  });
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
});
afterEach(() => {
  cleanup();
  client.clear();
});
function mount(children: React.ReactNode) {
  return render(
    <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  );
}

it.each([
  [undefined, "active", "Linked", true],
  [undefined, "revoked", "Disconnected", false],
  ["temporarily_unavailable", "active", "Temporarily unavailable", true],
  ["paused", "active", "Personal access paused", true],
  ["reconnect_required", "active", "Reconnect required", true],
  ["disconnected", "revoked", "Disconnected", false],
  ["available", "revoked", "Disconnected", false],
] as const)(
  "inventory distinguishes %s from saved %s consent",
  async (availability, consent, label, canRevoke) => {
    const chat = {
      platform: "discord",
      display_name: "Discord",
      login_command: "/nyx login",
      logout_command: "/nyx logout",
      status_command: "/nyx status",
      help_command: "/nyx help",
      question_instruction: 'Use /nyx ask question:"your question".',
      transport_disclosure: "Platform operated Discord chat.",
    };
    let revoked = false;
    vi.mocked(apiClient).mockImplementation(async (path) => {
      if (path === `/platform-personal-bindings/${binding}/revoke`) {
        revoked = true;
        return { revoked: true, work_may_continue_in_app: true };
      }
      if (path === "/platform-personal-bindings")
        return {
          bindings: [
            {
              chat,
              id: binding,
              generation: 1,
              manager: {
                id: "manager",
                display_name: "Shared bot",
                username: null,
              },
              sender: { id: "exact-native-id", display_name: "Person" },
              state:
                revoked ||
                (availability !== undefined &&
                  availability !== "available" &&
                  availability !== "paused")
                  ? "revoked"
                  : consent,
              ...(availability
                ? { consent_state: revoked ? "revoked" : consent }
                : {}),
              availability: revoked ? "disconnected" : availability,
              conversation_id: null,
              consented_at: new Date().toISOString(),
              revoked_at: null,
            },
          ],
          next_before_id: null,
        };
      throw new Error(`Unexpected fixture route ${path}`);
    });
    mount(<PlatformPersonalBindings />);
    expect(await screen.findByText(/Discord account/)).toHaveTextContent(label);
    if (label !== "Linked") {
      expect(
        screen.queryByText(/This creates its saved thread/),
      ).not.toBeInTheDocument();
      expect(screen.queryByText(/· Linked/)).not.toBeInTheDocument();
    }
    if (canRevoke) {
      if (availability === "temporarily_unavailable")
        expect(
          screen.getByText(/consent is retained and access can return/),
        ).toBeVisible();
      const user = userEvent.setup();
      await user.click(
        screen.getByRole("button", { name: "Disconnect personal access" }),
      );
      await user.click(screen.getByRole("button", { name: "Disconnect" }));
      await waitFor(() =>
        expect(
          screen.queryByRole("button", { name: "Disconnect personal access" }),
        ).not.toBeInTheDocument(),
      );
      expect(apiClient).toHaveBeenCalledWith(
        `/platform-personal-bindings/${binding}/revoke`,
        expect.objectContaining({ method: "POST", body: { generation: 1 } }),
      );
    } else {
      expect(
        screen.queryByRole("button", { name: "Disconnect personal access" }),
      ).not.toBeInTheDocument();
    }
  },
);
