import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import {
  useCatalogEntry,
  useCreateKey,
  useKeys,
  useKeyAuthorizationStatus,
} from "@/hooks/use-keys";
import { useInitiateOAuth } from "@/hooks/use-providers";
import { hardRedirect } from "@/lib/navigation";
import { validateHttpAuthorizationUrl } from "@/schemas/oauth-popup";
import {
  GOOGLE_WORKSPACE_SCOPES,
  GOOGLE_WORKSPACE_SLUG,
  hasWorkspaceAuthorization,
  isPersonalGoogleKey,
  readNyxbotProgress,
  saveNyxbotProgress,
  type NyxbotChannel,
  type NyxbotProgress,
} from "@/schemas/nyxbot-onboarding";

export function useNyxbotOnboarding(
  userId: string,
  referral: NyxbotChannel | undefined,
) {
  const [progress, setProgress] = useState(() => {
    const stored = readNyxbotProgress(userId);
    return { ...stored, channel: stored.channel ?? referral ?? null };
  });
  function updateProgress(patch: Partial<NyxbotProgress>) {
    const next = { ...progress, ...patch };
    saveNyxbotProgress(userId, next);
    setProgress(next);
  }
  const keys = useKeys();
  const catalog = useCatalogEntry(GOOGLE_WORKSPACE_SLUG);
  const authorization = useKeyAuthorizationStatus(progress.googleKeyId, true);
  const createKey = useCreateKey();
  const initiateOAuth = useInitiateOAuth();
  const pendingKey =
    authorization.data && isPersonalGoogleKey(authorization.data)
      ? authorization.data
      : undefined;
  const connectedKey =
    pendingKey && hasWorkspaceAuthorization(pendingKey)
      ? pendingKey
      : keys.data?.find(hasWorkspaceAuthorization);
  const entry = catalog.data;
  const scopesAllowed =
    !entry?.platform_scope_allowlist ||
    GOOGLE_WORKSPACE_SCOPES.every((scope) =>
      entry.platform_scope_allowlist?.includes(scope),
    );
  const googleAvailable = Boolean(
    entry?.provider_config_id &&
    entry.credential_mode !== "user" &&
    entry.has_platform_oauth_credentials &&
    scopesAllowed,
  );

  const connectGoogle = useMutation({
    mutationFn: async () => {
      if (!entry?.provider_config_id || !googleAvailable)
        throw new Error("Google Workspace authorization is unavailable.");
      const key =
        pendingKey ??
        (await createKey.mutateAsync({
          service_slug: entry.slug,
          label: "Nyxbot Google Workspace",
        }));
      updateProgress({ googleKeyId: key.id });
      const params = new URLSearchParams();
      if (progress.channel) params.set("channel", progress.channel);
      const result = await initiateOAuth.mutateAsync({
        providerId: entry.provider_config_id,
        keyId: key.id,
        scopeOverride: [
          ...new Set([
            ...(key.granted_scopes ?? []),
            ...GOOGLE_WORKSPACE_SCOPES,
          ]),
        ],
        redirectPath: `/onboarding${params.size ? `?${params}` : ""}`,
      });
      const url = validateHttpAuthorizationUrl(result.authorization_url);
      if (!url) throw new Error("Invalid Google authorization URL.");
      hardRedirect(url.href);
    },
  });
  return {
    progress,
    updateProgress,
    keys,
    catalog,
    authorization,
    connectedKey,
    googleAvailable,
    connectGoogle,
  };
}
