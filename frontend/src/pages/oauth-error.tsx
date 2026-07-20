import { useNavigate } from "@tanstack/react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ExternalLink, RotateCcw, ShieldAlert } from "lucide-react";
import { useApplyTheme } from "@/hooks/use-theme";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { ServiceIcon } from "@/components/service-icon";

const ERROR_LABELS: Record<string, string> = {
  invalid_request: "Invalid Request",
  invalid_redirect_uri: "Invalid Redirect URI",
  not_found: "Client Not Found",
  bad_request: "Bad Request",
  pkce_verification_failed: "PKCE Verification Failed",
  invalid_scope: "Invalid Scope",
  consent_required: "Consent Required",
  login_required: "Login Required",
  required_service_not_connected: "Connect a Required Service",
};

export function OAuthErrorPage() {
  useApplyTheme();
  const navigate = useNavigate();
  const search = new URLSearchParams(window.location.search);
  const code = search.get("code") ?? "unknown_error";
  const message =
    search.get("message") ??
    "An unexpected error occurred during authorization.";
  const serviceSlug = search.get("service_slug")?.trim() ?? "";
  const serviceName = search.get("service_name")?.trim() || serviceSlug;
  const isMissingRequiredService =
    code === "required_service_not_connected" && serviceSlug.length > 0;

  const title = ERROR_LABELS[code] ?? "Authorization Error";
  const connectServiceUrl = `/keys?tab=services&slug=${encodeURIComponent(serviceSlug)}`;

  return (
    <div
      className="flex min-h-dvh flex-col items-center justify-center bg-background p-4"
      style={{
        paddingTop: "max(1rem, var(--sat))",
        paddingBottom: "max(1rem, var(--sab))",
      }}
    >
      <div className="flex w-full max-w-[460px] flex-col items-center gap-8">
        <div className="flex items-center">
          <NyxidLogo className="h-9 w-auto" />
        </div>

        <Card className="w-full">
          <CardHeader className="space-y-3">
            <div className="flex h-8 w-8 items-center justify-center rounded-md bg-red-500/10">
              {isMissingRequiredService ? (
                <ServiceIcon slug={serviceSlug} size="xs" />
              ) : (
                <ShieldAlert className="h-4 w-4 text-red-400" />
              )}
            </div>
            <CardTitle>{title}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-[12px] leading-relaxed text-muted-foreground">
              {isMissingRequiredService
                ? `${serviceName} is required by this app but is not connected to your NyxID account. Connect it, then retry authorization.`
                : message}
            </p>
            {isMissingRequiredService && (
              <div className="flex items-center gap-3 rounded-lg border border-border bg-muted px-3 py-3">
                <ServiceIcon slug={serviceSlug} size="md" />
                <div className="min-w-0">
                  <p className="truncate text-xs font-medium text-foreground">
                    {serviceName}
                  </p>
                  <p className="truncate text-[11px] text-text-tertiary">
                    {serviceSlug}
                  </p>
                </div>
              </div>
            )}
            <div className="rounded-lg border border-border bg-muted px-3 py-2">
              <p className="text-[11px] text-text-tertiary">Error code</p>
              <p className="text-xs text-foreground">{code}</p>
            </div>
            {isMissingRequiredService ? (
              <div className="flex flex-col-reverse gap-2 pt-2 sm:flex-row sm:justify-end">
                <Button
                  variant="outline"
                  onClick={() => window.history.back()}
                  className="w-full sm:w-auto"
                >
                  <RotateCcw />
                  Retry authorization
                </Button>
                <Button asChild variant="primary" className="w-full sm:w-auto">
                  <a
                    href={connectServiceUrl}
                    target="_blank"
                    rel="noreferrer"
                  >
                    Connect service
                    <ExternalLink />
                  </a>
                </Button>
              </div>
            ) : (
              <div className="flex justify-end gap-3 pt-2">
                <Button variant="outline" onClick={() => window.history.back()}>
                  Go Back
                </Button>
                <Button onClick={() => void navigate({ to: "/dashboard" })}>
                  Home
                </Button>
              </div>
            )}
          </CardContent>
        </Card>

        <p className="text-center text-[11px] text-text-tertiary">
          If this issue persists, contact the application developer.
        </p>
      </div>
    </div>
  );
}
