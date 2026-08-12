import { afterEach, describe, expect, it, vi } from "vitest";
import { oauthChannelName, openOAuthPopup } from "./oauth-popup";

function stubWindowGeometry(geometry: Readonly<Record<string, number>>) {
  for (const [key, value] of Object.entries(geometry)) {
    vi.spyOn(window, key as "outerWidth", "get").mockReturnValue(value);
  }
}

describe("OAuth popup manager", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("opens synchronously with unique isolated window names", () => {
    const popup = {
      closed: false,
      close: vi.fn(),
      postMessage: vi.fn(),
    } as unknown as Window;
    const open = vi.spyOn(window, "open").mockReturnValue(popup);
    const first = openOAuthPopup();
    const second = openOAuthPopup();

    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    expect(open).toHaveBeenNthCalledWith(
      1,
      "/oauth-launching",
      expect.stringMatching(/^nyxid_oauth_/),
      expect.stringContaining("popup,width="),
    );
    expect(first?.launchId).not.toBe(second?.launchId);
    first?.close();
    second?.close();
  });

  it("centers the popup over the opener window, not the primary display", () => {
    const popup = {
      closed: false,
      close: vi.fn(),
      postMessage: vi.fn(),
    } as unknown as Window;
    const open = vi.spyOn(window, "open").mockReturnValue(popup);
    // A window on a second monitor to the right, taller than the popup.
    stubWindowGeometry({
      screenX: 1920,
      screenY: 100,
      outerWidth: 1400,
      outerHeight: 1000,
    });

    openOAuthPopup()?.close();

    expect(open).toHaveBeenCalledWith(
      "/oauth-launching",
      expect.any(String),
      "popup,width=760,height=820,left=2240,top=190",
    );
  });

  it("clamps the popup to a window smaller than its natural size", () => {
    const popup = {
      closed: false,
      close: vi.fn(),
      postMessage: vi.fn(),
    } as unknown as Window;
    const open = vi.spyOn(window, "open").mockReturnValue(popup);
    stubWindowGeometry({
      screenX: 0,
      screenY: 0,
      outerWidth: 600,
      outerHeight: 500,
    });

    openOAuthPopup()?.close();

    expect(open).toHaveBeenCalledWith(
      "/oauth-launching",
      expect.any(String),
      "popup,width=600,height=500,left=0,top=0",
    );
  });

  it("returns null when blocked and isolates channel names by nonce", () => {
    vi.spyOn(window, "open").mockReturnValue(null);
    expect(openOAuthPopup()).toBeNull();
    expect(oauthChannelName("8e1fcf2a-e679-4da2-9f54-2d90cd5f0085")).toBe(
      "nyxid.oauth.8e1fcf2a-e679-4da2-9f54-2d90cd5f0085",
    );
    expect(oauthChannelName("not-a-nonce")).toBeNull();
  });

  it("waits for an origin/source/id validated ready handshake before navigation", async () => {
    const nonce = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";
    const popup = {
      closed: false,
      close: vi.fn(),
      postMessage: vi.fn(),
    } as unknown as Window;
    vi.spyOn(window, "open").mockReturnValue(popup);
    const handle = openOAuthPopup();
    expect(handle).not.toBeNull();
    const navigation = handle!.navigate(
      `https://github.com/login/oauth/authorize?state=1cc_${nonce}`,
      nonce,
      "GitHub",
    );
    expect(popup.postMessage).not.toHaveBeenCalled();

    window.dispatchEvent(
      new MessageEvent("message", {
        origin: window.location.origin,
        source: popup,
        data: { type: "oauth_launch_ready", launchId: "wrong" },
      }),
    );
    expect(popup.postMessage).not.toHaveBeenCalled();
    window.dispatchEvent(
      new MessageEvent("message", {
        origin: window.location.origin,
        source: popup,
        data: { type: "oauth_launch_ready", launchId: handle!.launchId },
      }),
    );
    await navigation;
    expect(popup.postMessage).toHaveBeenCalledWith(
      {
        type: "oauth_launch_navigate",
        launchId: handle!.launchId,
        nonce,
        url: `https://github.com/login/oauth/authorize?state=1cc_${nonce}`,
        serviceName: "GitHub",
      },
      window.location.origin,
    );
    handle?.close();
  });

  it("allows five seconds for a cold interstitial before falling back", async () => {
    vi.useFakeTimers();
    const nonce = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";
    const popup = {
      closed: false,
      close: vi.fn(),
      postMessage: vi.fn(),
    } as unknown as Window;
    vi.spyOn(window, "open").mockReturnValue(popup);
    const handle = openOAuthPopup();
    let outcome: string | undefined;
    void handle
      ?.navigate(
        `https://github.com/login/oauth/authorize?state=1cc_${nonce}`,
        nonce,
      )
      .then(
        () => {
          outcome = "resolved";
        },
        (error: unknown) => {
          outcome = error instanceof Error ? error.message : "rejected";
        },
      );

    await vi.advanceTimersByTimeAsync(4_999);
    expect(outcome).toBeUndefined();
    await vi.advanceTimersByTimeAsync(1);
    expect(outcome).toBe("OAuth popup did not become ready");
    handle?.close();
  });
});
