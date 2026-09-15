import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppConnectStartPage } from "./app-connect-start";

const mocks = vi.hoisted(() => ({
  mfaRequired: false,
  flow: vi.fn(),
  mfa: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ ctx: "signed.context.token" }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: typeof mocks) => unknown) => selector(mocks),
}));
vi.mock("@/components/auth/auth-flow", () => ({
  AuthFlow: (props: unknown) => {
    mocks.flow(props);
    return <div>Embedded login</div>;
  },
}));
vi.mock("@/components/auth/mfa-verify-form", () => ({
  MfaVerifyForm: (props: unknown) => {
    mocks.mfa(props);
    return <div>Verify MFA</div>;
  },
}));
const metadata = {
  client_name: "App A",
  handoff_blurb: "<b>Your accounts</b>",
  logo_url: "/api/v1/branding/assets/09d0abe0-6f31-4c48-a582-a1c2e3f06836",
  homepage_url: "https://app.example",
  verified: false,
  destination: "app.example",
};
function mount() {
  return render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <AppConnectStartPage />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.mfaRequired = false;
  window.history.replaceState(
    {},
    "",
    "/connect/app/start/signed.context.token",
  );
  vi.stubGlobal(
    "fetch",
    vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify(metadata), { status: 200 }),
      ),
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
describe("app login handoff", () => {
  it("loads only the signed context and embeds same-origin continuation with an unverified logo", async () => {
    mount();
    expect(await screen.findByText("Embedded login")).toBeInTheDocument();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch).toHaveBeenCalledWith(
      "/oauth/authorize-context?ctx=signed.context.token",
      expect.objectContaining({
        credentials: "include",
        referrerPolicy: "no-referrer",
      }),
    );
    expect(JSON.stringify(vi.mocked(fetch).mock.calls)).not.toContain(
      "client_id",
    );
    expect(mocks.flow).toHaveBeenCalledWith(
      expect.objectContaining({
        returnTo: `${window.location.origin}/oauth/authorize-context/resume?ctx=signed.context.token`,
        preservePath: true,
      }),
    );
    expect(screen.getByRole("img", { name: "App A logo" })).toHaveAttribute(
      "src",
      metadata.logo_url,
    );
    expect(screen.queryByText("Verified")).not.toBeInTheDocument();
    expect(screen.getByText("<b>Your accounts</b>")).toBeInTheDocument();
    expect(
      screen.getByText("Secured by NyxID · app.example"),
    ).toBeInTheDocument();
  });
  it("keeps the shell and stored transaction through MFA", async () => {
    mocks.mfaRequired = true;
    mount();
    expect(await screen.findByText("Verify MFA")).toBeInTheDocument();
    expect(mocks.mfa).toHaveBeenCalledWith({
      returnTo: `${window.location.origin}/oauth/authorize-context/resume?ctx=signed.context.token`,
    });
    expect(mocks.flow).not.toHaveBeenCalled();
  });
  it("shows verification only when the context reports it", async () => {
    vi.mocked(fetch).mockResolvedValue(
      new Response(JSON.stringify({ ...metadata, verified: true })),
    );
    mount();
    expect(await screen.findByText("Verified")).toBeInTheDocument();
  });
  it("does not offer login for an expired, tampered, or consumed context", async () => {
    vi.mocked(fetch).mockResolvedValue(new Response("{}", { status: 404 }));
    mount();
    expect(
      await screen.findByText(/unavailable or has expired/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Restart sign-in from the app/)).toBeInTheDocument();
    expect(mocks.flow).not.toHaveBeenCalled();
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
  });
});
