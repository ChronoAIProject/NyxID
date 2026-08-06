import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OAUTH_LAUNCH_CONTEXT_KEY } from "@/lib/oauth-popup";
import { OAuthCompletePage } from "./oauth-complete";

const NONCE = "8e1fcf2a-e679-4da2-9f54-2d90cd5f0085";
const NEXT_NONCE = "23f1c824-960c-4b5c-8e12-71f1dbce5b20";

function setLaunchContext(nonce = NONCE) {
  sessionStorage.setItem(
    OAUTH_LAUNCH_CONTEXT_KEY,
    JSON.stringify({
      providerOrigin: "https://github.com",
      nonce,
      serviceName: "GitHub",
    }),
  );
}

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
    sessionStorage.clear();
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
      `/oauth-complete?status=complete&flow=cc&nonce=${NONCE}`,
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
    expect(window.location.pathname).toBe("/oauth-complete");
    expect(window.location.search).toBe("");
  });

  it("never auto-closes an error and exposes retry", () => {
    vi.useFakeTimers();
    setLaunchContext();
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=error&flow=cc&code=access_denied&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    expect(screen.getByText("GitHub: Authorization declined")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /try again/i }),
    ).toBeInTheDocument();
    vi.advanceTimersByTime(30_000);
    expect(window.close).not.toHaveBeenCalled();
  });

  it.each([
    ["provider_error", "Provider could not complete authorization"],
    ["state_expired", "Authorization expired"],
    ["state_replayed", "Authorization already used"],
    ["state_invalid", "Authorization is no longer valid"],
    ["session_mismatch", "Different NyxID account"],
    ["session_required", "NyxID session required"],
    ["exchange_failed", "Connection could not be saved"],
    ["server_error", "Connection could not be completed"],
  ] as const)("maps %s to specific failure copy", (code, title) => {
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=error&flow=cc&code=${code}&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    expect(screen.getByText(title)).toBeInTheDocument();
  });

  it("cancels success auto-close after interaction", () => {
    vi.useFakeTimers();
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=complete&flow=cc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    fireEvent.keyDown(window, { key: "Tab" });
    vi.advanceTimersByTime(3_500);
    expect(window.close).not.toHaveBeenCalled();
  });

  it("does not cancel success auto-close when the popup receives focus", () => {
    vi.useFakeTimers();
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=complete&flow=cc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    fireEvent.focus(window);
    vi.advanceTimersByTime(3_500);
    expect(window.close).toHaveBeenCalledTimes(1);
  });

  it("shows only a neutral, unverified success acknowledgement", () => {
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=complete&flow=cc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    expect(
      screen.getByText("Authorization response received"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/while the connection is verified/i),
    ).toBeInTheDocument();
    expect(screen.queryByText("Connection complete")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/credential is secured/i),
    ).not.toBeInTheDocument();
    expect(document.querySelector(".lucide-circle-check-big")).toBeNull();
  });

  it("names the service but never claims success for a trusted cc display context", () => {
    setLaunchContext();
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=complete&flow=cc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);

    expect(
      screen.getByText(/the GitHub connection's status appears there/i),
    ).toBeInTheDocument();
    expect(screen.queryByText(/authorized|connected/i)).not.toBeInTheDocument();
    expect(document.querySelector(".lucide-circle-check-big")).toBeNull();
  });

  it("ignores launch context when the nonce does not match", () => {
    setLaunchContext(NEXT_NONCE);
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=complete&flow=cc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);

    expect(screen.queryByText(/GitHub/)).not.toBeInTheDocument();
    expect(
      screen.getByText(/while the connection is verified/i),
    ).toBeInTheDocument();
  });

  it("ignores matching launch context outside the cc pilot", () => {
    setLaunchContext();
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=complete&flow=kc&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);

    expect(screen.queryByText(/GitHub/)).not.toBeInTheDocument();
    expect(
      screen.getByText(/while the connection is verified/i),
    ).toBeInTheDocument();
  });

  it("updates the cc launch-context nonce before retry navigation", async () => {
    setLaunchContext();
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=error&flow=cc&code=access_denied&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    fireEvent.click(screen.getByRole("button", { name: /try again/i }));

    MockBroadcastChannel.instances[0]?.emit({
      type: "oauth_retry",
      nextNonce: NEXT_NONCE,
      url: `https://github.com/login/oauth/authorize?state=1cc_${NEXT_NONCE}`,
    });

    expect(assign).toHaveBeenCalledWith(
      `https://github.com/login/oauth/authorize?state=1cc_${NEXT_NONCE}`,
    );
    expect(
      JSON.parse(sessionStorage.getItem(OAUTH_LAUNCH_CONTEXT_KEY) ?? ""),
    ).toEqual({
      providerOrigin: "https://github.com",
      nonce: NEXT_NONCE,
      serviceName: "GitHub",
    });
  });

  it("does not request a retry when the provider origin binding is missing", () => {
    window.history.replaceState(
      {},
      "",
      `/oauth-complete?status=error&flow=cc&code=access_denied&nonce=${NONCE}`,
    );
    render(<OAuthCompletePage />);
    expect(
      screen.getByText(/start the connection again there/i),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /try again/i }),
    ).not.toBeInTheDocument();
    expect(MockBroadcastChannel.instances[0]?.messages).not.toContainEqual({
      type: "oauth_action",
      action: "retry",
    });
  });

  it("direct open does not broadcast", () => {
    window.history.replaceState({}, "", "/oauth-complete");
    render(<OAuthCompletePage />);
    expect(screen.getByText("Nothing to complete")).toBeInTheDocument();
    expect(
      screen.getByText(/isn't part of an active connection attempt/i),
    ).toBeInTheDocument();
    expect(MockBroadcastChannel.instances).toHaveLength(0);
  });
});
