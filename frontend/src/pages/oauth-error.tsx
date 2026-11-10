import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useNavigate } from "@tanstack/react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ExternalLink, RotateCcw, ShieldAlert } from "lucide-react";
import { useApplyTheme } from "@/hooks/use-theme";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { ServiceIcon } from "@/components/service-icon";

const ERROR_LABELS: Record<string, string> = {
  get invalid_request() { return translateUi("Invalid Request"); },
  get invalid_redirect_uri() { return translateUi("Invalid Redirect URI"); },
  get not_found() { return translateUi("Client Not Found"); },
  get bad_request() { return translateUi("Bad Request"); },
  get pkce_verification_failed() { return translateUi("PKCE Verification Failed"); },
  get invalid_scope() { return translateUi("Invalid Scope"); },
  get consent_required() { return translateUi("Consent Required"); },
  get login_required() { return translateUi("Login Required"); },
  get required_service_not_connected() { return translateUi("Connect a Required Service"); },
};

export function OAuthErrorPage() {
  const { t } = useUiTranslation();
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
            <div className="flex h-8 w-8 items-center justify-center rounded-md bg-destructive/10">
              {isMissingRequiredService ? (
                <ServiceIcon slug={serviceSlug} size="xs" />
              ) : (
                <ShieldAlert className="h-4 w-4 text-destructive" />
              )}
            </div>
            <CardTitle>{title}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-12 leading-relaxed text-muted-foreground">
              {isMissingRequiredService
                ? t("{{serviceName}} is required by this app but is not connected to your NyxID account. Connect it, then retry authorization.", { serviceName: serviceName })
                : message}
            </p>
            {isMissingRequiredService && (
              <div className="flex items-center gap-3 rounded-lg border border-border bg-muted px-3 py-3">
                <ServiceIcon slug={serviceSlug} size="md" />
                <div className="min-w-0">
                  <p className="truncate text-xs font-medium text-foreground">
                    {serviceName}
                  </p>
                  <p className="truncate text-11 text-text-tertiary">
                    {serviceSlug}
                  </p>
                </div>
              </div>
            )}
            <div className="rounded-lg border border-border bg-muted px-3 py-2">
              <p className="text-11 text-text-tertiary">{t("Error code")}</p>
              <p className="text-xs text-foreground">{code}</p>
            </div>
            {isMissingRequiredService ? (
              <div className="flex flex-col-reverse gap-2 pt-2 sm:flex-row sm:justify-end">
                <Button
                  variant="outline"
                  onClick={() => window.history.back()}
                  className="w-full sm:w-auto"
                >
                  <RotateCcw />{t("Retry authorization")}</Button>
                <Button asChild variant="primary" className="w-full sm:w-auto">
                  <a
                    href={connectServiceUrl}
                    target="_blank"
                    rel="noreferrer"
                  >{t("Connect service")}<ExternalLink />
                  </a>
                </Button>
              </div>
            ) : (
              <div className="flex justify-end gap-3 pt-2">
                <Button variant="outline" onClick={() => window.history.back()}>{t("Go Back")}</Button>
                <Button onClick={() => void navigate({ to: "/dashboard" })}>{t("Home")}</Button>
              </div>
            )}
          </CardContent>
        </Card>

        <p className="text-center text-11 text-text-tertiary">{t("If this issue persists, contact the application developer.")}</p>
      </div>
    </div>
  );
}
