import { webcrypto } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AEVATAR_ORIGIN,
  clearAevatarAuthorization,
  getAevatarAuthorization,
} from "./nyxbot-aevatar-auth";
import { useAuthStore } from "@/stores/auth-store";

const { request, network } = vi.hoisted(() => ({
  request: vi.fn(),
  network: vi.fn(),
}));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  apiClient: request,
}));
const config = {
  baseUrl: "https://nyx-api.chrono-ai.fun",
  clientId: "8c76ced6-8f5a-4564-bea9-e3d98807f8ba",
};
const callback = `${AEVATAR_ORIGIN}/auto/callback`;
const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), { status });
let authorizationParams: URLSearchParams;
let redirectOverride: ((url: URL) => void) | undefined;
let subject: string;
let authError: string | undefined;

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("crypto", webcrypto);
  vi.stubGlobal("fetch", network);
  clearAevatarAuthorization();
  useAuthStore.setState({
    user: { id: "owner" } as NonNullable<
      ReturnType<typeof useAuthStore.getState>["user"]
    >,
    isAuthenticated: true,
  });
  subject = "owner";
  redirectOverride = undefined;
  authError = undefined;
  request.mockResolvedValue(config);
  network.mockImplementation(async (input: string, options?: RequestInit) => {
    if (input.startsWith("/oauth/authorize?")) {
      authorizationParams = new URL(input, "http://localhost").searchParams;
      if (authError) return json({ error: authError }, 403);
      const redirect = new URL(callback);
      redirect.searchParams.set("code", "test-authorization-code");
      redirect.searchParams.set("state", authorizationParams.get("state")!);
      redirectOverride?.(redirect);
      return json({ redirect_url: redirect.href });
    }
    if (input === "/oauth/token") {
      const form = options!.body as URLSearchParams;
      const challenge = Buffer.from(
        await webcrypto.subtle.digest(
          "SHA-256",
          new TextEncoder().encode(form.get("code_verifier")!),
        ),
      ).toString("base64url");
      expect(challenge).toBe(authorizationParams.get("code_challenge"));
      return json({
        access_token: "test-access",
        refresh_token: "discard-this-refresh",
        token_type: "Bearer",
        expires_in: 900,
      });
    }
    if (input === "/oauth/userinfo") return json({ sub: subject });
    throw new Error(`Unexpected request: ${input}`);
  });
});
afterEach(() => {
  clearAevatarAuthorization();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("Nyxbot Aevatar session authorization", () => {
  it("uses the current browser session, PKCE, and verified same-user bearer without a redirect", async () => {
    expect(await getAevatarAuthorization()).toBe("Bearer test-access");
    expect(request).toHaveBeenCalledWith(
      "/proxy/s/aevatar/api/auth/nyxid/config",
      expect.objectContaining({ preserveSessionOn401: true }),
    );
    expect(authorizationParams.get("scope")).toBe("openid profile email proxy");
    expect(authorizationParams.getAll("resource")).toEqual([
      `${config.baseUrl}/api/v1/proxy/s/aevatar`,
    ]);
    expect(authorizationParams.get("redirect_uri")).toBe(callback);
    expect(network).toHaveBeenCalledWith(
      expect.stringContaining("/oauth/authorize?"),
      expect.objectContaining({
        credentials: "include",
        headers: { Accept: "application/json" },
        redirect: "error",
      }),
    );
    expect(network).toHaveBeenCalledWith(
      "/oauth/token",
      expect.objectContaining({
        credentials: "omit",
        method: "POST",
        redirect: "error",
      }),
    );
    expect(network).toHaveBeenCalledWith(
      "/oauth/userinfo",
      expect.objectContaining({
        credentials: "omit",
        headers: { Authorization: "Bearer test-access" },
      }),
    );
    const count = network.mock.calls.length;
    await getAevatarAuthorization();
    expect(network).toHaveBeenCalledTimes(count);
    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
  });
  it("deduplicates concurrent authorization for the same account", async () => {
    await Promise.all([getAevatarAuthorization(), getAevatarAuthorization()]);
    expect(request).toHaveBeenCalledTimes(1);
    expect(network).toHaveBeenCalledTimes(3);
  });
  it("obtains fresh authorization after bounded cache expiry", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    await getAevatarAuthorization();
    vi.setSystemTime(Date.now() + 301000);
    await getAevatarAuthorization();
    expect(request).toHaveBeenCalledTimes(2);
  });
  it("honors missing consent without exchanging a token or submitting a decision", async () => {
    authError = "consent_required";
    await expect(getAevatarAuthorization()).rejects.toMatchObject({
      code: "channelConsentRequired",
    });
    expect(network).toHaveBeenCalledTimes(1);
  });
  it.each([
    (url: URL) => {
      url.hostname = "untrusted.example";
    },
    (url: URL) => {
      url.searchParams.set("state", "wrong");
    },
    (url: URL) => {
      url.searchParams.append("state", "duplicate");
    },
    (url: URL) => {
      url.searchParams.delete("code");
    },
    (url: URL) => {
      url.searchParams.set("iss", "https://untrusted.example");
    },
  ])(
    "rejects a mismatched OAuth response before token exchange",
    async (mutate) => {
      redirectOverride = mutate;
      await expect(getAevatarAuthorization()).rejects.toMatchObject({
        code: "channelAuthRequired",
      });
      expect(network).toHaveBeenCalledTimes(1);
    },
  );
  it("rejects tokens for another user", async () => {
    subject = "other-owner";
    await expect(getAevatarAuthorization()).rejects.toMatchObject({
      code: "channelAuthRequired",
    });
  });
  it("invalidates memory credentials on logout and fresh login", async () => {
    await getAevatarAuthorization();
    const user = useAuthStore.getState().user;
    useAuthStore.setState({ user: null, isAuthenticated: false });
    await expect(getAevatarAuthorization()).rejects.toMatchObject({
      code: "channelAuthRequired",
    });
    useAuthStore.setState({ user, isAuthenticated: true });
    await getAevatarAuthorization();
    expect(request).toHaveBeenCalledTimes(2);
  });
  it("rejects an in-flight grant after account changes", async () => {
    let resolve: ((value: typeof config) => void) | undefined;
    request.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const pending = getAevatarAuthorization();
    useAuthStore.setState({ user: null, isAuthenticated: false });
    resolve?.(config);
    await expect(pending).rejects.toMatchObject({
      code: "channelAuthRequired",
    });
  });
  it("does not expose credential-bearing transport errors", async () => {
    network.mockRejectedValue(new Error("secret-auth-code-or-token"));
    await expect(getAevatarAuthorization()).rejects.toMatchObject({
      message: "channelAuthRequired",
    });
  });
  it("rejects a different NyxID environment before using its OAuth client", async () => {
    request.mockResolvedValue({
      ...config,
      baseUrl: "https://untrusted.example",
    });
    await expect(getAevatarAuthorization()).rejects.toMatchObject({
      code: "channelAuthRequired",
    });
    expect(network).not.toHaveBeenCalled();
  });
});
