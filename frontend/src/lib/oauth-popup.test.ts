import { afterEach, describe, expect, it, vi } from "vitest";
import { oauthChannelName, openOAuthPopup } from "./oauth-popup";

describe("OAuth popup manager", () => {
  afterEach(() => vi.restoreAllMocks());

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
      "popup,width=760,height=820",
    );
    expect(first?.launchId).not.toBe(second?.launchId);
    first?.close();
    second?.close();
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
});
