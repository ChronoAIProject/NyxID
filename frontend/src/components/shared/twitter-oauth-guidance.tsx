import { Button } from "@/components/ui/button";
import { useRuntimeConfig } from "@/hooks/use-runtime-config";
import { copyToClipboard } from "@/lib/utils";
import { Copy, ExternalLink } from "lucide-react";
import { toast } from "sonner";

function isTwitterOAuthSlug(slug: string): boolean {
  return slug === "twitter" || slug === "api-twitter";
}

function providerCallbackUrl(apiBaseUrl: string | undefined): string | null {
  return apiBaseUrl ? `${apiBaseUrl}/api/v1/providers/callback` : null;
}

export function TwitterOAuthGuidance({
  slug,
}: {
  readonly slug: string;
}) {
  if (!isTwitterOAuthSlug(slug)) return null;

  return <TwitterOAuthGuidanceContent />;
}

function TwitterOAuthGuidanceContent() {
  const {
    data: runtimeConfig,
    isError,
    isLoading,
  } = useRuntimeConfig();
  const callbackUrl = providerCallbackUrl(runtimeConfig?.api_base_url);

  function handleCopy() {
    if (!callbackUrl) return;
    void copyToClipboard(callbackUrl).then(() => {
      toast.success("Callback URL copied");
    });
  }

  return (
    <div className="space-y-3 rounded-lg border border-border bg-muted/40 p-3">
      <div className="space-y-1">
        <p className="text-xs font-medium text-foreground">
          Twitter / X OAuth setup
        </p>
        <p className="text-xs text-muted-foreground">
          This integration requires an X app with OAuth 2.0 enabled in{" "}
          <strong>User authentication settings</strong> in X Developer Console.
          {callbackUrl
            ? " Configure the callback URL below as one of your app's redirect URIs."
            : " The exact callback URL is loaded from your NyxID backend and will appear below."}
        </p>
      </div>

      {callbackUrl ? (
        <div className="space-y-1.5">
          <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
            Callback URL
          </p>
          <div className="relative">
            <pre className="overflow-x-auto rounded-md bg-background p-2 pr-10 font-mono text-[11px] leading-relaxed">
              {callbackUrl}
            </pre>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              className="absolute right-1.5 top-1.5 h-7 w-7"
              onClick={handleCopy}
              aria-label="Copy callback URL"
            >
              <Copy className="h-3.5 w-3.5" aria-hidden="true" />
            </Button>
          </div>
        </div>
      ) : isLoading ? (
        <p className="rounded-md border border-border bg-background/60 p-2 text-xs text-muted-foreground">
          Loading callback URL...
        </p>
      ) : (
        <p className="rounded-md border border-warning/30 bg-warning/10 p-2 text-xs text-warning">
          {isError
            ? "Couldn't load callback URL. Please retry. If this persists, contact support."
            : "Callback URL not yet available. Please retry. If this persists, contact support."}
        </p>
      )}

      <a
        href="https://developer.x.com/en/portal/dashboard"
        target="_blank"
        rel="noopener noreferrer"
        className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
      >
        Where do I get Client ID and Client Secret? Open Keys &amp; Tokens in X
        Developer Console
        <ExternalLink className="h-3 w-3" />
      </a>
    </div>
  );
}
