import { beforeEach, describe, expect, it } from "vitest";
import { useOAuthPopupStore } from "./oauth-popup-store";

function attempt(launchId: string) {
  return { launchId, nonce: null, keyId: null, slug: "github", startedAt: 1 };
}

describe("OAuth popup store", () => {
  beforeEach(() => useOAuthPopupStore.setState({ attempt: null }));

  it("enforces one flow and ignores stale mutations", () => {
    expect(useOAuthPopupStore.getState().begin(attempt("launch-a"))).toBe(true);
    expect(useOAuthPopupStore.getState().begin(attempt("launch-b"))).toBe(
      false,
    );
    useOAuthPopupStore
      .getState()
      .setNonce("launch-a", "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085");
    useOAuthPopupStore.getState().end("launch-b");
    expect(useOAuthPopupStore.getState().attempt?.nonce).toBe(
      "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085",
    );
    useOAuthPopupStore.getState().end("launch-a");
    expect(useOAuthPopupStore.getState().attempt).toBeNull();
  });
});
