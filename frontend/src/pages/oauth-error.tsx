import { useNavigate } from "@tanstack/react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ShieldAlert } from "lucide-react";
import { useApplyTheme } from "@/hooks/use-theme";
import { NyxidLogo } from "@/components/brand/nyxid-logo";

const ERROR_LABELS: Record<string, string> = {
  invalid_request: "Invalid Request",
  invalid_redirect_uri: "Invalid Redirect URI",
  not_found: "Client Not Found",
  bad_request: "Bad Request",
  pkce_verification_failed: "PKCE Verification Failed",
  invalid_scope: "Invalid Scope",
  consent_required: "Consent Required",
  login_required: "Login Required",
};

export function OAuthErrorPage() {
  useApplyTheme();
  const navigate = useNavigate();
  const search = new URLSearchParams(window.location.search);
  const code = search.get("code") ?? "unknown_error";
  const message =
    search.get("message") ??
    "An unexpected error occurred during authorization.";

  const title = ERROR_LABELS[code] ?? "Authorization Error";

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
            <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-red-500/10">
              <ShieldAlert className="h-4 w-4 text-red-400" />
            </div>
            <CardTitle>{title}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-[12px] leading-relaxed text-muted-foreground">
              {message}
            </p>
            <div className="rounded-lg border border-border bg-muted px-3 py-2">
              <p className="text-[11px] text-text-tertiary">Error code</p>
              <p className="text-xs text-foreground">{code}</p>
            </div>
            <div className="flex justify-end gap-3 pt-2">
              <Button variant="outline" onClick={() => window.history.back()}>
                Go Back
              </Button>
              <Button onClick={() => void navigate({ to: "/dashboard" })}>Home</Button>
            </div>
          </CardContent>
        </Card>

        <p className="text-center text-[11px] text-text-tertiary">
          If this issue persists, contact the application developer.
        </p>
      </div>
    </div>
  );
}
