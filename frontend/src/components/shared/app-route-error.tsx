import { useEffect } from "react";
import { Link, useRouter, type ErrorComponentProps } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { NotFoundPage } from "@/components/shared/not-found-page";
import { useAuthStore } from "@/stores/auth-store";
import {
  isAssetLoadError,
  recoverFromAssetError,
  retryAfterAssetError,
} from "@/lib/chunk-recovery";

/**
 * Router-wide render-error fallback, wired as `defaultErrorComponent`.
 *
 * Without this, TanStack Router's CatchBoundary swallows any render error and
 * renders *nothing* — the entire app goes blank, including chrome the user
 * needs to navigate away (the assistant sidebar, the dashboard nav). One bad
 * field in one list then looks like every button in the app is dead. This
 * keeps the failure scoped: say what happened, and offer a way out.
 *
 * It also owns the second half of asset-load recovery. Setting
 * `defaultErrorComponent` gives every route its own CatchBoundary (see
 * `Match.js`: `routeErrorComponent ? CatchBoundary : SafeFragment`), which sits
 * *inside* any boundary at the root — so a failed lazy chunk lands here rather
 * than propagating upward. Routing that case through the shared coordinator is
 * what keeps the recovery reachable.
 */
export function AppRouteError({ error, reset }: ErrorComponentProps) {
  if (isAssetLoadError(error)) {
    return <AssetLoadError />;
  }
  return <RenderError error={error} reset={reset} />;
}

/**
 * Shown when part of the app could not be downloaded. The reload is attempted
 * first and silently: in the common case (a tab left open across a deploy) the
 * user only ever sees a refresh, never this screen. Reaching this UI means the
 * one reload allowed for this build has already been spent and the assets are
 * still unavailable — a stale edge cache, an offline client, or a bad response.
 *
 * The copy deliberately does not claim a new version was deployed. That is only
 * one of the causes, and asserting it is misleading for the others.
 */
function AssetLoadError() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);

  // In an effect, never in render: recovery navigates away, and render must stay
  // free of side effects (React may render this more than once, e.g. StrictMode).
  // The coordinator is idempotent, so overlapping with the `vite:preloadError`
  // path is harmless — whichever runs first wins and the other gets "exhausted".
  useEffect(() => {
    recoverFromAssetError();
  }, []);

  return (
    <NotFoundPage
      code="Offline"
      title="Couldn't load part of the app"
      description="Some of the app's files didn't download. This usually clears on its own — try again, or head back and come at it from another angle."
      action={
        <div className="flex flex-wrap items-center justify-center gap-2">
          <Button
            type="button"
            variant="primary"
            onClick={() => retryAfterAssetError()}
          >
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

/** Everything that is a genuine render error rather than a missing asset. */
function RenderError({ error, reset }: ErrorComponentProps) {
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
