import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { getRouteApi } from "@tanstack/react-router";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";

const route = getRouteApi("/connector-continuation/$id");

export function ConnectorContinuationPage() {
  const { t } = useUiTranslation();
  const { id } = route.useParams();
  const { user, isAuthenticated, isLoading } = useAuthStore();
  const [confirming, setConfirming] = useState(false);
  const [confirmationFailed, setConfirmationFailed] = useState(false);
  const [appReturn, setAppReturn] = useState<string>();
  const endpoint = `/providers/callback/continuation/${encodeURIComponent(id)}`;
  const preview = useQuery({
    queryKey: ["connector-continuation", id, user?.id],
    queryFn: () => api.post<{ provider: string; account: string }>(endpoint, {}),
    enabled: isAuthenticated,
    retry: false,
    refetchOnWindowFocus: false,
  });

  async function confirm() {
    setConfirming(true);
    setConfirmationFailed(false);
    try {
      const result = await api.post<{ return_url: string }>(`${endpoint}/confirm`, {
        confirm: true,
      });
      const destination = new URL(result.return_url);
      if (destination.origin === window.location.origin) {
        window.location.assign(destination.href);
      } else if (
        destination.origin === "null" &&
        destination.hostname === "services" &&
        destination.pathname === "/oauth-return" &&
        !["javascript:", "data:", "file:"].includes(destination.protocol)
      ) {
        setAppReturn(destination.href);
        window.location.assign(destination.href);
      } else {
        throw new Error("Invalid connector return URL");
      }
    } catch {
      setConfirmationFailed(true);
      setConfirming(false);
    }
  }

  const loginPath = `/login?return_to=${encodeURIComponent(window.location.origin + window.location.pathname)}`;
  return (
    <main className="flex min-h-screen items-center justify-center px-4">
      <div className="flex max-w-sm flex-col items-center gap-4 text-center">
        <NyxidIcon className="h-12 w-12" />
        {isLoading || (isAuthenticated && preview.isPending) ? (
          <p role="status">{t("Loading connection request…")}</p>
        ) : !isAuthenticated ? (
          <>
            <h1 className="type-dialog-title">{t("Sign in to continue")}</h1>
            <p className="type-body text-muted-foreground">
              {t("Use the NyxID account that started this connection.")}
            </p>
            <Button asChild><a href={loginPath}>{t("Sign in")}</a></Button>
          </>
        ) : appReturn ? (
          <Button asChild><a href={appReturn}>{t("Return to the app")}</a></Button>
        ) : preview.data ? (
          <>
            <h1 className="type-dialog-title">
              {t("Connect {{provider}} to {{account}}?", preview.data)}
            </h1>
            <Button disabled={confirming} onClick={() => void confirm()}>
              {confirming ? t("Connecting…") : t("Confirm connection")}
            </Button>
            {confirmationFailed && (
              <p role="alert">{t("Unable to confirm the connection. Restart the request to try again.")}</p>
            )}
          </>
        ) : (
          <>
            <h1 className="type-dialog-title">{t("Connection request unavailable")}</h1>
            <p className="type-body text-muted-foreground">
              {t("This request has expired or belongs to another NyxID account.")}
            </p>
            <Button asChild variant="outline"><a href="/dashboard">{t("Go to Dashboard")}</a></Button>
          </>
        )}
      </div>
    </main>
  );
}
