import { Link, useRouter, type ErrorComponentProps } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { NotFoundPage } from "@/components/shared/not-found-page";
import { useAuthStore } from "@/stores/auth-store";

/**
 * Router-wide render-error fallback, wired as `defaultErrorComponent`.
 *
 * Without this, TanStack Router's CatchBoundary swallows any render error and
 * renders *nothing* — the entire app goes blank, including chrome the user
 * needs to navigate away (the assistant sidebar, the dashboard nav). One bad
 * field in one list then looks like every button in the app is dead. This
 * keeps the failure scoped: say what happened, and offer a way out.
 *
 * Reuses the empty-state visual language of `NotFoundPage` (DESIGN.md) so the
 * two failure surfaces read as one system.
 */
export function AppRouteError({ error, reset }: ErrorComponentProps) {
  const router = useRouter();
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);

  // `reset` clears the boundary; `invalidate` re-runs the route's loaders so a
  // retry refetches instead of replaying the same failed render.
  function retry() {
    reset();
    void router.invalidate();
  }

  return (
    <NotFoundPage
      code="Error"
      title="Something broke on this page"
      description={
        import.meta.env.DEV && error instanceof Error && error.message
          ? error.message
          : "This part of the app hit an unexpected error. Your data is safe — try again, or head back and come at it from another angle."
      }
      action={
        <div className="flex flex-wrap items-center justify-center gap-2">
          <Button type="button" variant="primary" onClick={retry}>
            Try again
          </Button>
          <Button asChild variant="secondary">
            {isAuthenticated ? (
              <Link to="/dashboard">Back to dashboard</Link>
            ) : (
              <Link to="/">Go to homepage</Link>
            )}
          </Button>
        </div>
      }
    />
  );
}
