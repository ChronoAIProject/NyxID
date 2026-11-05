import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { WhatsappRollbackTokenSection } from "./whatsapp-rollback-token";

const mocks = vi.hoisted(() => ({ post: vi.fn(), get: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: mocks }));

describe("WhatsApp rollback token replacement", () => {
  beforeEach(() => vi.clearAllMocks());

  it("issues on the owner action, shows once, and keeps the secret out of query/mutation data", async () => {
    const token = "new-verify-token-once";
    mocks.post.mockResolvedValue({ verify_token: token, relay_webhook_url: "https://nyx.example/api/v1/webhooks/channel/whatsapp/bot", revision: 3, previous_token_invalidated_immediately: true });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const view = render(<QueryClientProvider client={client}><WhatsappRollbackTokenSection botId="bot" revision={2} /></QueryClientProvider>);
    expect(mocks.post).not.toHaveBeenCalled();
    expect(screen.getByText(/immediately invalidates the old relay Verify Token/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Generate replacement Verify Token" }));
    await waitFor(() => expect(screen.getByText(token)).toBeInTheDocument());
    expect(mocks.post).toHaveBeenCalledExactlyOnceWith("/channel-bots/bot/native-whatsapp/relay-verify-token", { revision: 2 });
    expect(screen.getByText("https://nyx.example/api/v1/webhooks/channel/whatsapp/bot")).toBeInTheDocument();
    expect(screen.getByText(/Point Meta’s callback back/)).toBeInTheDocument();
    await waitFor(() => expect(client.getMutationCache().getAll()[0]?.state.status).toBe("success"));
    expect(client.getMutationCache().getAll()[0]?.state.data).toBeUndefined();
    expect(client.getMutationCache().getAll()[0]?.state.variables).toBe(2);
    expect(client.getQueryCache().getAll()).toHaveLength(0);
    // A settings refetch updates revision without losing or issuing the secret again.
    view.rerender(<QueryClientProvider client={client}><WhatsappRollbackTokenSection botId="bot" revision={3} /></QueryClientProvider>);
    expect(screen.getByText(token)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "I have saved it" }));
    await waitFor(() => expect(screen.queryByText(token)).not.toBeInTheDocument());
    await waitFor(() => expect(client.getMutationCache().getAll()).toHaveLength(0));
    expect(mocks.get).not.toHaveBeenCalled();
    expect(mocks.post).toHaveBeenCalledTimes(1);
    client.clear();
  });

  it("reports a refused replacement without opening a secret dialog or retrying automatically", async () => {
    mocks.post.mockRejectedValue(new Error("settings_changed_reload"));
    const client = new QueryClient();
    render(<QueryClientProvider client={client}><WhatsappRollbackTokenSection botId="bot" revision={1} /></QueryClientProvider>);
    fireEvent.click(screen.getByRole("button", { name: "Generate replacement Verify Token" }));
    await waitFor(() => expect(screen.getByText("settings_changed_reload")).toBeInTheDocument());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(mocks.post).toHaveBeenCalledTimes(1);
    client.clear();
  });
});
