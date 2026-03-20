import { useEffect, useMemo, useRef } from "react";
import { useNavigate } from "@tanstack/react-router";
import { CheckCircle, LoaderCircle, XCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useCompleteTelegramConnect } from "@/hooks/use-providers";
import { ApiError } from "@/lib/api-client";
import { parseTelegramCallbackSearch } from "@/lib/telegram-login";

/**
 * Callback page that reads status from URL query params.
 *
 * The backend OAuth callback handler (GET /api/v1/providers/callback)
 * redirects here with ?status=success or ?status=error&message=...
 */
export function ProvidersCallbackPage() {
  const navigate = useNavigate();
  const completeTelegramConnect = useCompleteTelegramConnect();
  const hasSubmittedTelegramCallback = useRef(false);
  const search = useMemo(
    () => new URLSearchParams(window.location.search),
    [],
  );
  const telegramCallback = useMemo(
    () => parseTelegramCallbackSearch(search),
    [search],
  );

  useEffect(() => {
    if (
      !telegramCallback.isTelegramCallback ||
      telegramCallback.payload === null ||
      hasSubmittedTelegramCallback.current
    ) {
      return;
    }

    hasSubmittedTelegramCallback.current = true;
    completeTelegramConnect.mutate({
      providerId: telegramCallback.payload.providerId,
      data: telegramCallback.payload.data,
    });
  }, [completeTelegramConnect, telegramCallback]);

  if (telegramCallback.isTelegramCallback) {
    const telegramError =
      telegramCallback.error ??
      (completeTelegramConnect.error instanceof ApiError
        ? completeTelegramConnect.error.message
        : completeTelegramConnect.isError
          ? "Telegram connection failed"
          : null);

    if (
      telegramError === null &&
      (completeTelegramConnect.isPending || !hasSubmittedTelegramCallback.current)
    ) {
      return (
        <div className="flex items-center justify-center py-16">
          <Card className="w-full max-w-md">
            <CardHeader className="text-center">
              <CardTitle>Verifying Telegram Login</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col items-center gap-4">
              <LoaderCircle className="h-12 w-12 animate-spin text-primary" />
              <p className="text-center text-sm text-muted-foreground">
                Finishing your Telegram connection.
              </p>
            </CardContent>
          </Card>
        </div>
      );
    }

    return (
      <div className="flex items-center justify-center py-16">
        <Card className="w-full max-w-md">
          <CardHeader className="text-center">
            <CardTitle>
              {telegramError === null
                ? "Telegram Connected"
                : "Connection Failed"}
            </CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col items-center gap-4">
            {telegramError === null ? (
              <>
                <CheckCircle className="h-12 w-12 text-success" />
                <p className="text-center text-sm text-muted-foreground">
                  Your Telegram account has been connected successfully.
                </p>
                <Button onClick={() => void navigate({ to: "/providers" })}>
                  Back to Providers
                </Button>
              </>
            ) : (
              <>
                <XCircle className="h-12 w-12 text-destructive" />
                <p className="text-center text-sm text-destructive">
                  {telegramError}
                </p>
                <Button
                  variant="outline"
                  onClick={() => void navigate({ to: "/providers" })}
                >
                  Back to Providers
                </Button>
              </>
            )}
          </CardContent>
        </Card>
      </div>
    );
  }

  const isSuccess = search.get("status") === "success";
  const errorMessage =
    search.get("status") === "error"
      ? (search.get("message") ?? "OAuth connection failed")
      : "Missing callback parameters";

  return (
    <div className="flex items-center justify-center py-16">
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <CardTitle>
            {isSuccess ? "Provider Connected" : "Connection Failed"}
          </CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col items-center gap-4">
          {isSuccess ? (
            <>
              <CheckCircle className="h-12 w-12 text-success" />
              <p className="text-sm text-muted-foreground text-center">
                Your provider has been connected successfully.
              </p>
              <Button onClick={() => void navigate({ to: "/providers" })}>
                Back to Providers
              </Button>
            </>
          ) : (
            <>
              <XCircle className="h-12 w-12 text-destructive" />
              <p className="text-sm text-destructive text-center">
                {errorMessage}
              </p>
              <Button
                variant="outline"
                onClick={() => void navigate({ to: "/providers" })}
              >
                Back to Providers
              </Button>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
