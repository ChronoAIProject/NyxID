import { useRuntimeConfig } from "@/hooks/use-runtime-config";
import { CopyableUrlCallout } from "@/components/shared/copyable-url-callout";
import { ExternalLink } from "lucide-react";

function isTwitterOAuthSlug(slug: string): boolean {
  return slug === "twitter" || slug === "api-twitter";
}

function providerCallbackUrl(apiBaseUrl: string | undefined): string | null {
  return apiBaseUrl ? `${apiBaseUrl}/api/v1/providers/callback` : null;
}

/**
 * Renders the NyxID callback / redirect URL that the user must register
 * in their OAuth provider's developer console, for ANY authorization-code
 * OAuth flow (GitHub, Google, Lark, Twitter, ...). Without this, the user
 * has no way to know which redirect URI to whitelist on the provider side.
 *
 * Callers are responsible for only rendering this for OAuth /
 * authorization-code flows — it has no meaning for device-code,
 * API-key, bearer, header, or no-auth credential types, none of which
 * use a redirect URI. The `provider_type === "oauth2"` field is the
 * signal both dialog call sites already use to route flows; the wizard's
 * `OAuthFlow` is by construction the authorization-code flow (its sibling
 * `DeviceCodeFlow` handles device codes), so it always renders this.
 *
 * Twitter / X gets an extra guidance block layered on top because its
 * Developer Console wording (User authentication settings, Keys & Tokens)
 * is non-obvious and worth spelling out.
 *
 * @deprecated Use {@link CopyableUrlCallout} directly for new surfaces.
 * This component is kept as a thin shim so existing OAuth provider
 * create/edit paths migrate to the shared primitive automatically. The
 * export name + `slug` prop are preserved so no caller changes are
 * required.
 */
export function OAuthCallbackGuidance({
  slug,
}: {
  readonly slug: string;
}) {
  const {
    data: runtimeConfig,
    isError,
    isLoading,
  } = useRuntimeConfig();
  const callbackUrl = providerCallbackUrl(runtimeConfig?.api_base_url);
  const isTwitter = isTwitterOAuthSlug(slug);

  if (!callbackUrl) {
    if (isLoading) {
      return (
        <p className="rounded-md border border-border bg-background/60 p-2 text-xs text-muted-foreground">
          Loading callback URL...
        </p>
      );
    }
    return (
      <p className="rounded-md border border-warning/30 bg-warning/10 p-2 text-xs text-warning">
        {isError
          ? "Couldn't load callback URL. Please retry. If this persists, contact support."
          : "Callback URL not yet available. Please retry. If this persists, contact support."}
      </p>
    );
  }

  const heading = isTwitter ? "Twitter / X OAuth setup" : "NyxID callback URL";
  const description =
    isTwitter
      ? "This integration requires an X app with OAuth 2.0 enabled in User authentication settings in X Developer Console. Configure the callback URL below as one of your app's redirect URIs."
      : "Add this URL as an authorized redirect URI in your OAuth app's settings on the provider's developer console, or authorization will fail.";

  return (
    <div className="space-y-2">
      <CopyableUrlCallout
        label={heading}
        url={callbackUrl}
        description={description}
      />
      {isTwitter ? (
        <a
          href="https://developer.x.com/en/portal/dashboard"
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
        >
          Where do I get Client ID and Client Secret? Open Keys &amp; Tokens in
          X Developer Console
          <ExternalLink className="h-3 w-3" />
        </a>
      ) : null}
    </div>
  );
}
