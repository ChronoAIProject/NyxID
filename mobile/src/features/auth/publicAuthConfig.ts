import { localizedError } from "../../i18n/localizedError";
import { z } from "zod";
import { isSocialProvider, socialProviders, type SocialProvider } from "../../lib/auth/socialProviders";

const publicAuthConfigSchema = z.object({
  email_auth_enabled: z.boolean(),
  social_providers: z.array(z.string()),
});

export type PublicAuthConfig = {
  emailAuthEnabled: boolean;
  socialProviders: SocialProvider[];
};

export function parsePublicAuthConfig(payload: unknown): PublicAuthConfig {
  const result = publicAuthConfigSchema.safeParse(payload);
  if (!result.success) {
    // Error feedback must never include the response body or validation input.
    throw localizedError("Unable to load sign-in options. Please try again.");
  }

  return {
    emailAuthEnabled: result.data.email_auth_enabled,
    socialProviders: [
      ...new Set(result.data.social_providers.filter(isSocialProvider)),
    ],
  };
}

export function requireSocialProvider(
  config: PublicAuthConfig | null | undefined,
  provider: SocialProvider,
): void {
  if (!config) {
    throw localizedError("Sign-in options are not ready. Please wait or retry.");
  }
  if (!config.socialProviders.includes(provider)) {
    const label = socialProviders.find(({ id }) => id === provider)?.name ?? provider;
    throw localizedError("{{label}} sign-in is not enabled on this server. Choose another available sign-in method.", { label: label});
  }
}
