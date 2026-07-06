import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { NotFoundPage } from "@/components/shared/not-found-page";
import { useAuthStore } from "@/stores/auth-store";

/**
 * Desktop-app 404, wired as the router's `defaultNotFoundComponent`.
 * Wraps the shared `NotFoundPage` with a router-aware home button:
 * signed-in users go to the dashboard, everyone else to the landing page.
 */
export function AppNotFound() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  return (
    <NotFoundPage
      action={
        <Button asChild variant="primary">
          {isAuthenticated ? (
            <Link to="/dashboard">Back to dashboard</Link>
          ) : (
            <Link to="/">Go home</Link>
          )}
        </Button>
      }
    />
  );
}
