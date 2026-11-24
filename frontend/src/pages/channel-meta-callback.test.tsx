import { StrictMode } from "react";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { ChannelMetaCallbackPage } from "./channel-meta-callback";
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: { user: { id: string } }) => unknown) =>
    select({ user: { id: "actor" } }),
}));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it.each(["cancelled", "revoked"])(
  "exchanges once under StrictMode and shows %s consent accurately",
  async (status) => {
    window.history.replaceState(
      null,
      "",
      `/channel-bots/meta-callback/instagram?state=${status}`,
    );
    const post = vi
      .spyOn(api, "post")
      .mockResolvedValue({
        status,
        session_id: "11111111-1111-4111-8111-111111111111",
        platform: "instagram",
        assets: [],
        bot_id: null,
        expires_at: "2026-12-01T00:00:00Z",
      });
    const close = vi.spyOn(window, "close").mockImplementation(() => undefined);
    render(
      <StrictMode>
        <ChannelMetaCallbackPage />
      </StrictMode>,
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        status === "cancelled"
          ? "Sign-in was cancelled"
          : "Inbox consent was revoked",
      ),
    );
    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith("/channel-bots/meta/instagram/exchange", {
      state: status,
      code: null,
    });
    expect(close).not.toHaveBeenCalled();
    expect(window.location.search).toBe("");
  },
);
