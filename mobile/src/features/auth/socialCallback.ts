export const SOCIAL_CALLBACK_URL = "nyxid://auth/social/callback";

export type SocialProvider = "google" | "github" | "apple";

type SocialCallback = {
  status: "success" | "error";
  accessToken?: string;
  refreshToken?: string;
  expiresIn?: number;
  error?: string;
  provider?: SocialProvider;
};

// Expo launch URLs and unrelated account/device links are not social replies.
// Only the exact native callback route may affect the social sign-in state.
export function parseSocialCallback(url: string): SocialCallback | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (
    parsed.protocol !== "nyxid:" ||
    parsed.hostname !== "auth" ||
    parsed.pathname !== "/social/callback" ||
    parsed.port ||
    parsed.username ||
    parsed.password
  ) {
    return null;
  }

  const statusRaw = parsed.searchParams.get("status");
  const providerRaw = parsed.searchParams.get("provider");
  const provider: SocialProvider | undefined =
    providerRaw === "google" ||
    providerRaw === "github" ||
    providerRaw === "apple"
      ? providerRaw
      : undefined;
  if (statusRaw !== "success" && statusRaw !== "error") {
    return { status: "error", error: "social_auth_unknown", provider };
  }
  if (statusRaw === "error") {
    return {
      status: "error",
      error:
        parsed.searchParams.get("error") ??
        parsed.searchParams.get("message") ??
        undefined,
      provider,
    };
  }

  const expiresInRaw = parsed.searchParams.get("expires_in");
  const expiresInParsed = expiresInRaw ? Number(expiresInRaw) : NaN;
  return {
    status: "success",
    accessToken: parsed.searchParams.get("access_token") ?? undefined,
    refreshToken: parsed.searchParams.get("refresh_token") ?? undefined,
    expiresIn:
      Number.isFinite(expiresInParsed) && expiresInParsed > 0
        ? expiresInParsed
        : undefined,
    provider,
  };
}
