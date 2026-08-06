import { render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OAUTH_LAUNCH_CONTEXT_KEY } from "@/lib/oauth-popup";
import { OAuthLaunchingPage } from "./oauth-launching";

const LAUNCH_ID = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";
const NONCE = "23f1c824-960c-4b5c-8e12-71f1dbce5b20";

describe("OAuth launching interstitial", () => {
  afterEach(() => {
    window.name = "";
    sessionStorage.clear();
    Object.defineProperty(window, "opener", {
      configurable: true,
      writable: true,
      value: null,
    });
    vi.restoreAllMocks();
  });

  it("severs its opener before acknowledging readiness", async () => {
    const postMessage = vi.fn(() => {
      expect(window.opener).toBeNull();
    });
    const opener = { postMessage } as unknown as Window;
    window.name = `nyxid_oauth_${LAUNCH_ID}`;
    Object.defineProperty(window, "opener", {
      configurable: true,
      writable: true,
      value: opener,
    });

    render(<OAuthLaunchingPage />);

    await waitFor(() =>
      expect(postMessage).toHaveBeenCalledWith(
        { type: "oauth_launch_ready", launchId: LAUNCH_ID },
        window.location.origin,
      ),
    );
    expect(window.opener).toBeNull();
  });

  it("does not acknowledge an invalid launch window", () => {
    const postMessage = vi.fn();
    window.name = "nyxid_oauth_forged";
    Object.defineProperty(window, "opener", {
      configurable: true,
      writable: true,
      value: { postMessage } as unknown as Window,
    });

    render(<OAuthLaunchingPage />);

    expect(postMessage).not.toHaveBeenCalled();
    expect(window.opener).toBeNull();
  });

  it("navigates only with the parsed, protocol-approved authorization URL", async () => {
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    const opener = { postMessage: vi.fn() } as unknown as Window;
    window.name = `nyxid_oauth_${LAUNCH_ID}`;
    Object.defineProperty(window, "opener", {
      configurable: true,
      writable: true,
      value: opener,
    });
    render(<OAuthLaunchingPage />);

    window.dispatchEvent(
      new MessageEvent("message", {
        origin: window.location.origin,
        source: opener,
        data: {
          type: "oauth_launch_navigate",
          launchId: LAUNCH_ID,
          nonce: NONCE,
          url: `javascript:alert(1)?state=1cc_${NONCE}`,
        },
      }),
    );
    expect(assign).not.toHaveBeenCalled();

    const providerUrl = `https://github.com/login/oauth/authorize?state=1cc_${NONCE}`;
    window.dispatchEvent(
      new MessageEvent("message", {
        origin: window.location.origin,
        source: opener,
        data: {
          type: "oauth_launch_navigate",
          launchId: LAUNCH_ID,
          nonce: NONCE,
          url: providerUrl,
          serviceName: "GitHub",
        },
      }),
    );

    await waitFor(() => expect(assign).toHaveBeenCalledWith(providerUrl));
    expect(
      JSON.parse(sessionStorage.getItem(OAUTH_LAUNCH_CONTEXT_KEY) ?? ""),
    ).toEqual({
      providerOrigin: "https://github.com",
      nonce: NONCE,
      serviceName: "GitHub",
    });
  });

  it("drops an invalid service label without blocking navigation", async () => {
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    const opener = { postMessage: vi.fn() } as unknown as Window;
    window.name = `nyxid_oauth_${LAUNCH_ID}`;
    Object.defineProperty(window, "opener", {
      configurable: true,
      writable: true,
      value: opener,
    });
    render(<OAuthLaunchingPage />);

    const providerUrl = `https://github.com/login/oauth/authorize?state=1cc_${NONCE}`;
    window.dispatchEvent(
      new MessageEvent("message", {
        origin: window.location.origin,
        source: opener,
        data: {
          type: "oauth_launch_navigate",
          launchId: LAUNCH_ID,
          nonce: NONCE,
          url: providerUrl,
          serviceName: "x".repeat(65),
        },
      }),
    );

    await waitFor(() => expect(assign).toHaveBeenCalledWith(providerUrl));
    expect(
      JSON.parse(sessionStorage.getItem(OAUTH_LAUNCH_CONTEXT_KEY) ?? ""),
    ).toEqual({
      providerOrigin: "https://github.com",
      nonce: NONCE,
    });
  });
});
