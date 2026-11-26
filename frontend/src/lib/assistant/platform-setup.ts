import { z } from "zod";
import { api } from "@/lib/api-client";
import {
  getAssistantIdentityRevision,
  getAssistantIdentityUserId,
} from "./identity";
import { nyxAgentTransport } from "./nyxagent-transport";
import { isAdmittedSendError } from "./send-admission";

export const platformSetupCompletionSchema = z
  .object({
    kind: z.enum(["agent_key", "channel_bot"]),
    id: z.string().uuid(),
  })
  .strict();
export type PlatformSetupCompletion = z.infer<
  typeof platformSetupCompletionSchema
>;

const resources = {
  agent_key: { path: "/api-keys/", label: "Agent Key" },
  channel_bot: { path: "/channel-bots/", label: "channel bot" },
} as const;

/** A user finishes setup; read back metadata, then continue the exact source chat. */
export async function completePlatformSetup(
  result: PlatformSetupCompletion,
  source: {
    readonly ownerUserId: string;
    readonly identityRevision: number;
    readonly conversationId: string;
  },
): Promise<void> {
  const completion = platformSetupCompletionSchema.parse(result);
  const guard = () => {
    if (
      getAssistantIdentityRevision() !== source.identityRevision ||
      getAssistantIdentityUserId() !== source.ownerUserId
    ) {
      throw new Error("Your account changed. Reopen setup before continuing.");
    }
  };
  guard();
  const resource = resources[completion.kind];
  const live = z
    .object({ id: z.string().uuid() })
    .parse(
      await api.get(`${resource.path}${completion.id}`, {
        authorityGuard: guard,
      }),
    );
  guard();
  if (live.id !== completion.id)
    throw new Error(
      "NyxID could not verify the setup result. Refresh and try again.",
    );
  // Only metadata reaches chat. One-time keys and webhook secrets stay in
  // their existing first-party result screens.
  const text = `I completed setup for ${resource.label} ${live.id} in NyxID. Verify its current state and permissions, then continue the task we were discussing.`;
  await new Promise<void>((resolve, reject) => {
    void nyxAgentTransport
      .send(source.conversationId, text, () => resolve(), undefined, {
        adoptExisting: true,
      })
      .then(() => resolve())
      .catch((error: unknown) =>
        isAdmittedSendError(error) ? resolve() : reject(error),
      );
  });
}
