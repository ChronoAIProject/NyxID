import { useNavigate, useSearch } from "@tanstack/react-router";
import { CheckCircle, XCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

/**
 * Callback page that reads status from URL query params.
 *
 * The backend OAuth callback handler (GET /api/v1/providers/callback)
 * redirects here with ?status=success or ?status=error&message=...
 */
export function ProvidersCallbackPage() {
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as {
    readonly status?: string;
    readonly message?: string;
  };

  const isSuccess = search.status === "success";
  const errorMessage =
    search.status === "error"
      ? (search.message ?? "OAuth connection failed")
      : null;

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
              <CheckCircle className="h-12 w-12 text-emerald-500" />
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
