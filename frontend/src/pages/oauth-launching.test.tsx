import { render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OAuthLaunchingPage } from "./oauth-launching";

const LAUNCH_ID = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";

describe("OAuth launching interstitial", () => {
  afterEach(() => {
    window.name = "";
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
});
