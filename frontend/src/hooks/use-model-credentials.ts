import { useState } from "react";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite, hasAdminRead } from "@/types/api";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  credentialPageSchema,
  credentialSchema,
} from "@/schemas/model-credentials";
export const credentialPath = "/admin/platform-models/credentials";
export const offeringPath = "/admin/platform-models/offerings";
export function useModelCredentials(offering = false) {
  const user = useAuthStore((s) => s.user);
  return useInfiniteQuery({
    queryKey: [
      "platform-models",
      offering ? "offerings" : "credentials",
      user?.id,
    ],
    initialPageParam: "",
    queryFn: async ({ pageParam }) =>
      credentialPageSchema.parse(
        await api.get(
          `${offering ? offeringPath : credentialPath}${pageParam ? `?after=${encodeURIComponent(pageParam)}` : ""}`,
        ),
      ),
    getNextPageParam: (page) => page.next_after ?? undefined,
    enabled: hasAdminRead(user),
    refetchOnWindowFocus: false,
  });
}
/** Secrets never enter the query/mutation cache, error objects, or notifications. */
export function useCredentialSave() {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const user = useAuthStore((s) => s.user);
  const enabled = useFeature(FEATURE_FLAG.INFERENCE_PLATFORM_MODELS);
  const client = useQueryClient();
  async function save(path: string, body: unknown, update = false) {
    if (!enabled || !canAdminWrite(user))
      throw new Error("Platform models are read-only.");
    setPending(true);
    setError(undefined);
    try {
      const result = credentialSchema.parse(
        await (update ? api.put(path, body) : api.post(path, body)),
      );
      await client.invalidateQueries({ queryKey: ["platform-models"] });
      return result;
    } catch (err) {
      // Do not render arbitrary response text from a credential submission.
      setError(
        err instanceof ApiError && err.status === 409
          ? "The credential changed or an enabled route still references it. Reload the inventory before trying again."
          : "Could not save the credential. Check the input and reload the inventory before retrying; the save may already have completed.",
      );
      return undefined;
    } finally {
      setPending(false);
    }
  }
  return { save, pending, error };
}
