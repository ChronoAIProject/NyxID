import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { ChannelBotSetup } from "./channel-bot-setup";
import type { ChannelBotConnectProps } from "./channel-bot-connect";
import { ChannelConnectLinkContext } from "@/hooks/use-channel-connect-link";
import { channelPlatformViews } from "@/lib/channel-platforms";
import { platformFixtures } from "@/test/fixtures/channel-platforms";

const mocks = vi.hoisted(() => ({ toastError: vi.fn(), fetch: vi.fn() }));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError, success: vi.fn() } }));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  Link: ({ children }: { children: React.ReactNode }) => <span>{children}</span>,
}));
vi.mock("@/lib/telemetry", () => ({ isTelemetryActive: () => false }));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: [] }) }));
vi.mock("@/components/shared/org-scope-select", () => ({ OrgScopeSelect: () => null }));
vi.mock("@/hooks/use-channel-platforms", () => ({
  useChannelPlatformViews: () => {
    const platforms = channelPlatformViews(platformFixtures);
    return { data: { platforms: platformFixtures }, platforms, getPlatform: (id: string) => platforms[id] };
  },
}));
vi.mock("./channel-bot-connect", () => ({
  ChannelBotConnect: ({ renderFields, children }: ChannelBotConnectProps) => (
    <>{renderFields({ disabled: false })}{children}</>
  ),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it.each([false, true].flatMap((hosted) => [400, 429, 502].map((status) => ({ hosted, status }))))(
  "shows only the NyxID message for HTTP $status (hosted: $hosted)",
  async ({ hosted, status }) => {
    const message = status === 400
      ? "Discord rejected the bot token. Copy it from Developer Portal > Bot > Reset Token."
      : status === 429 ? "Rate limited" : "Discord users/@me failed (HTTP 500)";
    await submitRejectedBot(hosted, new Response(JSON.stringify({
      error: "channel_error", error_code: 10005, message, extra: "private provider body",
    }), { status, headers: { "content-type": "application/json" } }), message);
  },
);

it.each([false, true])("never dumps a non-JSON edge-proxy error (hosted: %s)", async (hosted) => {
  await submitRejectedBot(hosted, new Response("<html>private provider body</html>", { status: 502 }), "Request failed with status 502");
});

async function submitRejectedBot(hosted: boolean, response: Response, message: string) {
  vi.stubGlobal("fetch", mocks.fetch);
  mocks.fetch.mockResolvedValueOnce(response);
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false }, queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ChannelConnectLinkContext.Provider value={hosted ? { id: "link", token: "nyx_bcl_fixture", refresh: async () => undefined } : null}>
        <ChannelBotSetup open onOpenChange={vi.fn()} defaultOrgId={null} defaultPlatform="discord" defaultLabel="Support" fullPage={hosted} />
      </ChannelConnectLinkContext.Provider>
    </QueryClientProvider>,
  );
  fireEvent.change(screen.getByLabelText("Bot token"), { target: { value: "private-token" } });
  fireEvent.change(screen.getByLabelText("public_key"), { target: { value: "a".repeat(64) } });
  const submit = screen.getByRole("button", { name: hosted ? "Create channel bot" : "Add Bot" });
  await waitFor(() => expect(submit).toBeEnabled());
  fireEvent.click(submit);
  await waitFor(() => expect(mocks.toastError).toHaveBeenCalledExactlyOnceWith(message));
  expect(mocks.fetch).toHaveBeenCalledWith(
    hosted ? "/api/v1/channel-connect-links/complete" : "/api/v1/channel-bots",
    expect.objectContaining({ method: "POST" }),
  );
  expect(mocks.toastError.mock.calls[0]).not.toContain("private provider body");
  client.clear();
}
