import { isMfaChallengeToken } from "../../lib/auth/mfaChallenge";
export type SocialProvider = "google" | "github" | "apple";
export const SOCIAL_CALLBACK_URL = "nyxid://auth/social/callback";
type SocialCallback = {
  status: "success" | "error" | "mfa_required";
  accessToken?: string;
  refreshToken?: string;
  expiresIn?: number;
  error?: string;
  provider?: SocialProvider;
  mfaToken?: string;
};

export function parseSocialCallback(url: string): SocialCallback | null {
  if (!url.startsWith(SOCIAL_CALLBACK_URL)) {
    return null;
  }

  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "nyxid:" || parsed.host !== "auth" || parsed.pathname !== "/social/callback" || parsed.username || parsed.password) return null;
    // MFA challenges are returned in the fragment so the raw token is not
    // sent as a query parameter or Referer. It remains in memory only.
    const fragmentParams = new URLSearchParams(parsed.hash.replace(/^#/, ""));
    const statusRaw = parsed.searchParams.get("status") ?? fragmentParams.get("status");
    const providerRaw = parsed.searchParams.get("provider") ?? fragmentParams.get("provider");
    const provider: SocialProvider | undefined =
      providerRaw === "google" || providerRaw === "github" || providerRaw === "apple"
        ? providerRaw
        : undefined;
    if (statusRaw === "mfa_required") {
      const token = fragmentParams.get("mfa_token");
      if (!isMfaChallengeToken(token)) {
        return { status: "error", error: "social_auth_invalid_mfa", provider };
      }
      return {
        status: "mfa_required",
        provider,
        mfaToken: token,
      };
    }
    if (statusRaw !== "success" && statusRaw !== "error") {
      return {
        status: "error",
        error: "social_auth_unknown",
        provider,
      };
    }

    if (statusRaw === "error") {
      return {
        status: "error",
        error: parsed.searchParams.get("error") ?? parsed.searchParams.get("message") ?? undefined,
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
        Number.isFinite(expiresInParsed) && expiresInParsed > 0 ? expiresInParsed : undefined,
      provider,
    };
  } catch {
    return {
      status: "error",
      error: "social_auth_unknown",
    };
  }
}
