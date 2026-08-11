import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DirectChatControls,
  DirectModeBanner,
  DIRECT_MODE_COPY,
} from "@/components/assistant/direct-chat-controls";
import { api } from "@/lib/api-client";
import { directAssistantTransport } from "@/lib/assistant/direct-transport";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

const owner: User = {
  id: "direct-controls-user",
  email: "direct@example.com",
  display_name: "Direct User",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-08-11T00:00:00.000Z",
};

function renderWithQuery(ui: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
  useAuthStore.getState().setUser(null);
  useAuthStore.getState().setUser(owner);
});

describe("DirectChatControls", () => {
  it("loads first-party catalogs with plain GETs and retains draft selections", async () => {
    const get = vi.spyOn(api, "get").mockImplementation(async (endpoint) => {
      if (endpoint === "/assistant/direct/models") {
        return [
          { id: "gpt-5.5", label: "GPT-5.5", default: true },
          { id: "gpt-5.2", label: "GPT-5.2", default: false },
        ] as never;
      }
      return [
        { slug: "nyxid", label: "NyxID" },
        { slug: "github-via-nyxid", label: "GitHub via NyxID" },
      ] as never;
    });
    const event = userEvent.setup();
    renderWithQuery(<DirectChatControls conversationId={undefined} />);

    await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
    expect(get).toHaveBeenCalledWith("/assistant/direct/models");
    expect(get).toHaveBeenCalledWith("/assistant/direct/skills");
    expect(
      screen.getByText(
        "A skill teaches the model about NyxID; it cannot take actions here.",
      ),
    ).toBeVisible();

    await event.click(screen.getByRole("combobox", { name: "Model" }));
    await event.click(screen.getByRole("option", { name: "GPT-5.2" }));
    await event.click(
      screen.getByRole("combobox", { name: "NyxID knowledge" }),
    );
    await event.click(screen.getByRole("option", { name: "NyxID" }));

    expect(directAssistantTransport.getSettings()).toEqual({
      model: "gpt-5.2",
      skillSlug: "nyxid",
    });
    const conversation = await directAssistantTransport.createConversation();
    expect(directAssistantTransport.getSettings(conversation.id)).toEqual({
      model: "gpt-5.2",
      skillSlug: "nyxid",
    });
  });
});

describe("DirectModeBanner", () => {
  it("shows the required advisory copy and persists dismissal", async () => {
    const event = userEvent.setup();
    const view = render(<DirectModeBanner />);
    expect(screen.getByText(DIRECT_MODE_COPY)).toBeVisible();

    await event.click(
      screen.getByRole("button", { name: "Dismiss direct chat notice" }),
    );
    expect(screen.queryByText(DIRECT_MODE_COPY)).not.toBeInTheDocument();

    view.unmount();
    render(<DirectModeBanner />);
    expect(screen.queryByText(DIRECT_MODE_COPY)).not.toBeInTheDocument();
  });
});
