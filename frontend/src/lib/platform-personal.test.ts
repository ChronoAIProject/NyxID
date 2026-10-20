import { beforeEach, expect, it, vi } from "vitest";
import { personalLinkApi, personalActorGuard } from "./platform-personal";
import { useAuthStore } from "@/stores/auth-store";
import { apiClient, ApiError } from "./api-client";
vi.mock("./api-client", async (original) => ({
  ...(await original<typeof import("./api-client")>()),
  apiClient: vi.fn(),
}));
const challenge = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const result = {
  challenge_id: challenge,
  status: "awaiting_chat_confirmation",
  expires_at: "2026-10-15T10:00:00Z",
  binding: null,
  confirmation_command: "/confirm P-ABCD-EFGH",
};
beforeEach(() => {
  vi.mocked(apiClient).mockReset();
});

it("carries credentials in POST bodies and never recovers a confirmation command through status", async () => {
  vi.mocked(apiClient).mockResolvedValue(result);
  expect(
    (await personalLinkApi.approve(challenge, "digest", false, "owner"))
      .confirmation_command,
  ).toBe("/confirm P-ABCD-EFGH");
  expect(apiClient).toHaveBeenLastCalledWith(
    "/platform-personal-links/approve",
    expect.objectContaining({
      method: "POST",
      body: {
        challenge_id: challenge,
        review_digest: "digest",
        replace_existing: false,
      },
    }),
  );
  expect(
    (await personalLinkApi.status(challenge, "owner")).confirmation_command,
  ).toBeNull();
  expect(apiClient).toHaveBeenLastCalledWith(
    "/platform-personal-links/status",
    expect.objectContaining({
      method: "POST",
      body: { challenge_id: challenge },
    }),
  );
});

it("blocks responses or actions for a different captured account", () => {
  const guard = personalActorGuard("old-owner");
  useAuthStore.setState({ user: { id: "new-owner" } as never });
  expect(guard).toThrow("Your account changed");
});

it("sanitizes echoed server errors so tokens and commands do not reach diagnostics or toasts", async () => {
  vi.mocked(apiClient).mockRejectedValue(
    new ApiError(409, {
      error: "conflict",
      error_code: 1,
      message: "secret intent_token echoed /confirm P-ABCD-EFGH",
    }),
  );
  await expect(personalLinkApi.review("secret", "owner")).rejects.toThrow(
    "This review changed.",
  );
});
