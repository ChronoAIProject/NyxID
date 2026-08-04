import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OAuthCompletePage } from "./oauth-complete";

const NONCE = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";

class MockBroadcastChannel {
  static instances: MockBroadcastChannel[] = [];
  readonly name: string;
  readonly messages: unknown[] = [];
  private listeners: Array<(event: MessageEvent<unknown>) => void> = [];
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
  constructor(name: string) {
    this.name = name;
    MockBroadcastChannel.instances.push(this);
  }
  postMessage(message: unknown) {
    this.messages.push(message);
  }
  close() {}
  addEventListener(
    _type: string,
    listener: (event: MessageEvent<unknown>) => void,
  ) {
    this.listeners.push(listener);
  }
  removeEventListener(
    _type: string,
    listener: (event: MessageEvent<unknown>) => void,
  ) {
    this.listeners = this.listeners.filter((item) => item !== listener);
  }
  emit(message: unknown) {
    const event = new MessageEvent("message", { data: message });
    this.onmessage?.(event);
    for (const listener of this.listeners) listener(event);
  }
}

describe("OAuth completion page", () => {
  beforeEach(() => {
    MockBroadcastChannel.instances = [];
    vi.stubGlobal("BroadcastChannel", MockBroadcastChannel);
    vi.spyOn(window, "close").mockImplementation(() => undefined);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("broadcasts a nonce-free wakeup and scrubs the URL", () => {
    window.history.replaceState(
      {},
      "",
      `/oauth?status=complete&flow=cc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    const channel = MockBroadcastChannel.instances[0];
    expect(channel?.name).toBe(`nyxid.oauth.${NONCE}`);
    expect(channel?.messages[0]).toEqual({
      type: "oauth_result",
      status: "complete",
      flow: "cc",
    });
    expect(channel?.messages[0]).not.toHaveProperty("nonce");
    expect(window.location.pathname).toBe("/oauth");
    expect(window.location.search).toBe("");
  });

  it("never auto-closes an error and exposes retry", () => {
    vi.useFakeTimers();
    window.history.replaceState(
      {},
      "",
      `/oauth?status=error&flow=cc&code=access_denied&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    expect(screen.getByText("Authorization declined")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /try again/i }),
    ).toBeInTheDocument();
    vi.advanceTimersByTime(30_000);
    expect(window.close).not.toHaveBeenCalled();
  });

  it("cancels success auto-close after interaction", () => {
    vi.useFakeTimers();
    window.history.replaceState(
      {},
      "",
      `/oauth?status=complete&flow=cc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    fireEvent.keyDown(window, { key: "Tab" });
    vi.advanceTimersByTime(3_500);
    expect(window.close).not.toHaveBeenCalled();
  });

  it("direct open does not broadcast", () => {
    window.history.replaceState({}, "", "/oauth");
    render(<OAuthCompletePage />);
    expect(screen.getByText("Nothing to complete")).toBeInTheDocument();
    expect(MockBroadcastChannel.instances).toHaveLength(0);
  });
});
