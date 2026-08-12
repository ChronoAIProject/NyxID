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

function renderWithQuery(
  ui: React.ReactNode,
  queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  }),
) {
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
          { id: "gpt-5.5", label: "GPT-5.5", default: false },
          { id: "gpt-5.4", label: "GPT-5.4", default: true },
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
    await waitFor(() =>
      expect(directAssistantTransport.getSettings()).toEqual({
        model: "gpt-5.4",
        skillSlug: null,
      }),
    );

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

  it("falls back to the first published model when none is marked default", async () => {
    vi.spyOn(api, "get").mockImplementation(async (endpoint) => {
      if (endpoint === "/assistant/direct/models") {
        return [
          { id: "server-first", label: "Server First", default: false },
          { id: "server-second", label: "Server Second", default: false },
        ] as never;
      }
      return [] as never;
    });

    renderWithQuery(<DirectChatControls conversationId={undefined} />);

    await waitFor(() =>
      expect(directAssistantTransport.getSettings().model).toBe("server-first"),
    );
  });

  it("does not replace an explicit model when the catalog arrives", async () => {
    directAssistantTransport.setModel(undefined, "user-choice");
    const get = vi.spyOn(api, "get").mockImplementation(async (endpoint) => {
      if (endpoint === "/assistant/direct/models") {
        return [
          { id: "server-default", label: "Server Default", default: true },
          { id: "user-choice", label: "User Choice", default: false },
        ] as never;
      }
      return [] as never;
    });

    renderWithQuery(<DirectChatControls conversationId={undefined} />);

    await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
    await waitFor(() => {
      expect(screen.getByRole("combobox", { name: "Model" })).toHaveTextContent(
        "User Choice",
      );
      expect(directAssistantTransport.getSettings().model).toBe("user-choice");
    });
  });

  it("renders but disables picker writes when the direct conversation is missing", () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    queryClient.setQueryData(
      ["assistant", "direct", "models"],
      [
        { id: "gpt-5.5", label: "Server Default", default: true },
        { id: "gpt-5.4", label: "Other Model", default: false },
      ],
    );
    queryClient.setQueryData(
      ["assistant", "direct", "skills"],
      [{ slug: "nyxid", label: "NyxID" }],
    );
    vi.spyOn(api, "get").mockResolvedValue([] as never);
    expect(() =>
      renderWithQuery(
        <DirectChatControls conversationId="direct-missing" />,
        queryClient,
      ),
    ).not.toThrow();

    expect(screen.getByRole("combobox", { name: "Model" })).toBeDisabled();
    expect(
      screen.getByRole("combobox", { name: "NyxID knowledge" }),
    ).toBeDisabled();

    expect(directAssistantTransport.getSettings("direct-missing")).toEqual({
      model: "gpt-5.5",
      skillSlug: null,
    });
  });

  it("disables draft picker controls while signed out", () => {
    useAuthStore.getState().setUser(null);
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    queryClient.setQueryData(
      ["assistant", "direct", "models"],
      [{ id: "gpt-5.5", label: "Server Default", default: true }],
    );
    vi.spyOn(api, "get").mockResolvedValue([] as never);

    expect(() =>
      renderWithQuery(
        <DirectChatControls conversationId={undefined} />,
        queryClient,
      ),
    ).not.toThrow();

    expect(screen.getByRole("combobox", { name: "Model" })).toBeDisabled();
    expect(
      screen.getByRole("combobox", { name: "NyxID knowledge" }),
    ).toBeDisabled();
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
