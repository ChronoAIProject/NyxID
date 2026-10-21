import { z } from "zod";
import { apiClient, ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  personalLinkPreviewSchema,
  personalLinkReviewSchema,
  personalLinkStatusSchema,
  personalBindingsSchema,
} from "@/schemas/platform-personal";

export function telegramConfirmationUrl(
  command: string | undefined,
  username: string | null | undefined,
) {
  if (!command || !username || !/^[A-Za-z0-9_]{5,32}$/.test(username))
    return undefined;
  const code = /^\/confirm (P-[A-Z0-9]{4}-[A-Z0-9]{4})$/.exec(command)?.[1];
  return code
    ? `https://t.me/${username}?start=nyxpersonal_${code}`
    : undefined;
}

export function personalActorGuard(actor: string | undefined) {
  return () => {
    if (!actor || useAuthStore.getState().user?.id !== actor)
      throw new Error("Your account changed. Review again before continuing.");
  };
}
async function request<T>(
  path: string,
  schema: z.ZodType<T>,
  body: unknown,
  actor?: string,
) {
  try {
    return schema.parse(
      await apiClient(path, {
        method: "POST",
        body,
        ...(actor ? { authorityGuard: personalActorGuard(actor) } : {}),
      }),
    );
  } catch (error) {
    if (error instanceof ApiError && error.status === 401)
      throw new Error("Sign in again to continue this request.");
    if (error instanceof ApiError && error.status === 409)
      throw new Error(
        "This review changed. Review the request again before continuing.",
      );
    if (
      error instanceof ApiError &&
      (error.status === 404 || error.status === 410)
    )
      throw new Error(
        "This request is unavailable or expired. Open the bot's private chat help and start a new login request.",
      );
    if (error instanceof ApiError && error.status === 403)
      throw new Error(
        "This account cannot link right now. Complete account verification and check that personal access is enabled for this bot.",
      );
    if (error instanceof ApiError && error.status === 429)
      throw new Error(
        "Too many linking requests. Wait a moment before trying again.",
      );
    throw new Error(
      "Could not complete this request. Check your connection and try again.",
    );
  }
}
export const personalLinkApi = {
  preview: (token: string) =>
    request("/platform-personal-links/preview", personalLinkPreviewSchema, {
      intent_token: token,
    }),
  review: (token: string, actor: string) =>
    request(
      "/platform-personal-links/review",
      personalLinkReviewSchema,
      { intent_token: token },
      actor,
    ),
  approve: (
    challengeId: string,
    digest: string,
    replace: boolean,
    actor: string,
  ) =>
    request(
      "/platform-personal-links/approve",
      personalLinkStatusSchema,
      {
        challenge_id: challengeId,
        review_digest: digest,
        replace_existing: replace,
      },
      actor,
    ),
  status: async (challengeId: string, actor: string) => {
    const status = await request(
      "/platform-personal-links/status",
      personalLinkStatusSchema,
      { challenge_id: challengeId },
      actor,
    );
    // Status never recreates a one-time approval command, even on a bad server response.
    return { ...status, confirmation_command: null };
  },
  deny: (challengeId: string, actor: string) =>
    request(
      "/platform-personal-links/deny",
      z.object({ status: z.literal("denied") }),
      { challenge_id: challengeId },
      actor,
    ),
};
export async function loadPersonalBindings(actor: string, before?: string) {
  return personalBindingsSchema.parse(
    await apiClient(
      `/platform-personal-bindings${before ? `?before_id=${encodeURIComponent(before)}` : ""}`,
      { authorityGuard: personalActorGuard(actor) },
    ),
  );
}
export async function revokePersonalBinding(
  actor: string,
  id: string,
  generation: number,
) {
  return request(
    `/platform-personal-bindings/${id}/revoke`,
    z.object({
      revoked: z.literal(true),
      work_may_continue_in_app: z.literal(true),
    }),
    { generation },
    actor,
  );
}
