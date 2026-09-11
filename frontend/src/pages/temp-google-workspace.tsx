import { useState } from "react";
import {
  ArrowRight,
  CheckCircle2,
  ExternalLink,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { WebDeviceLogin } from "@/components/auth/web-device-login";
import {
  AddKeyDialog,
  type AuthorizationAttempt,
} from "@/components/dashboard/add-key-dialog";
import { Button } from "@/components/ui/button";
import {
  useCatalogEntry,
  useKeys,
  useKeyAuthorizationWatch,
} from "@/hooks/use-keys";
import { useAuthStore } from "@/stores/auth-store";
import { TempConfiguredOAuthReturn } from "@/pages/temp-configured-oauth-return";

const GOOGLE_CATALOG_SLUG = "api-google";
const WORKSPACE_PERMISSIONS = [
  {
    name: "Google Drive",
    scope: "https://www.googleapis.com/auth/drive.file",
    detail: "Files created or explicitly opened with this app",
  },
  {
    name: "Gmail",
    scope: "https://www.googleapis.com/auth/gmail.readonly",
    detail: "Read email messages",
  },
  {
    name: "Google Calendar",
    scope: "https://www.googleapis.com/auth/calendar.readonly",
    detail: "Read calendars and events",
  },
] as const;

export function TempGoogleWorkspacePage() {
  const authenticated = useAuthStore((state) => state.isAuthenticated);
  const loading = useAuthStore((state) => state.isLoading);
  const user = useAuthStore((state) => state.user);
  const returnTo = `${window.location.origin}/temp${window.location.search}`;

  return (
    <main className="min-h-screen bg-background text-foreground">
      <div className="mx-auto max-w-5xl px-6 py-10 sm:px-10 sm:py-16">
        <header className="mb-14 flex flex-wrap items-center justify-between gap-4 border-b border-border pb-5">
          <a href="/" className="text-lg font-semibold tracking-tight">
            NyxID <span className="text-muted-foreground">/ Connect lab</span>
          </a>
          <span className="rounded-full border border-amber-500/30 bg-amber-500/10 px-3 py-1 font-mono text-xs text-amber-500">
            LOCAL PAGE · LIVE ACCOUNT
          </span>
        </header>

        <div className="mb-12 max-w-2xl">
          <p className="mb-4 font-mono text-xs uppercase tracking-[0.2em] text-muted-foreground">
            Google Workspace / OAuth proof of concept
          </p>
          <h1 className="text-4xl font-semibold tracking-tight sm:text-5xl">
            Your Google account.
            <br />
            <span className="text-muted-foreground">
              Connected through NyxID.
            </span>
          </h1>
          <p className="mt-6 max-w-xl text-base leading-7 text-muted-foreground">
            Sign in, review the permissions available from NyxID’s Google app,
            and authorize in a separate tab. This page checks the saved
            connection when you return.
          </p>
        </div>

        <section
          aria-labelledby="login-heading"
          className="grid gap-6 border-t border-border py-8 sm:grid-cols-[180px_1fr]"
        >
          <div>
            <p className="mb-2 font-mono text-xs text-muted-foreground">
              01 / IDENTITY
            </p>
            <h2 id="login-heading" className="text-lg font-medium">
              Sign in to NyxID
            </h2>
          </div>
          <div>
            {loading ? (
              <p
                role="status"
                className="flex items-center gap-2 text-muted-foreground"
              >
                <Loader2 className="size-4 animate-spin" />
                Checking your session…
              </p>
            ) : authenticated ? (
              <div className="flex items-center gap-3">
                <CheckCircle2 className="size-5 text-emerald-500" />
                <div>
                  <p className="font-medium">
                    Signed in{user?.email ? ` as ${user.email}` : ""}
                  </p>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Connections created here are saved to your real personal
                    account.
                  </p>
                </div>
              </div>
            ) : (
              <div className="max-w-md space-y-4">
                <p className="text-sm leading-6 text-muted-foreground">
                  Approve the login in your NyxID mobile app. The QR flow
                  returns directly to this local page.
                </p>
                <WebDeviceLogin returnTo={returnTo} />
                <a
                  className="inline-flex items-center gap-2 text-sm underline underline-offset-4"
                  href={`/login?return_to=${encodeURIComponent(returnTo)}`}
                >
                  Use the login page <ArrowRight className="size-3" />
                </a>
                <p className="text-xs leading-5 text-muted-foreground">
                  Email and password also work locally. Google social sign-in
                  uses a separate callback and currently returns to the hosted
                  site.
                </p>
              </div>
            )}
          </div>
        </section>

        {authenticated ? (
          <>
            <GoogleBinding key={user?.id ?? "session"} />
            {user && (
              <TempConfiguredOAuthReturn
                key={`return-${user.id}`}
                userId={user.id}
              />
            )}
          </>
        ) : (
          <section className="border-t border-border py-8">
            <p className="text-sm text-muted-foreground">
              Sign in to load the live Google app configuration and your
              existing connections.
            </p>
          </section>
        )}

        <footer className="mt-10 border-t border-border pt-5 text-xs leading-6 text-muted-foreground">
          Keep this page open during Google authorization. If the Google tab
          finishes on the hosted NyxID site, return here to check the
          connection.
        </footer>
      </div>
    </main>
  );
}

function GoogleBinding() {
  const catalog = useCatalogEntry(GOOGLE_CATALOG_SLUG);
  const keys = useKeys();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [attempt, setAttempt] = useState<
    (AuthorizationAttempt & { deadlineAt: number }) | null
  >(null);
  const [aborted, setAborted] = useState(false);
  const watch = useKeyAuthorizationWatch(attempt?.keyId ?? null, {
    attemptId: attempt?.attemptId ?? "not-started",
    previousAuthorizationAt: attempt?.previousAuthorizationAt,
    enabled: attempt !== null && !aborted,
    deadlineAt: attempt?.deadlineAt ?? 0,
  });
  const entry = catalog.data;
  const managedReady =
    entry?.provider_type === "oauth2" &&
    Boolean(entry.provider_config_id) &&
    entry.has_platform_oauth_credentials === true &&
    (entry.credential_mode === "both" || entry.credential_mode === "admin");
  const googleKeys =
    keys.data?.filter(
      (key) => key.catalog_service_slug === GOOGLE_CATALOG_SLUG,
    ) ?? [];
  const allowlist = entry?.platform_scope_allowlist;

  function refresh() {
    void catalog.refetch();
    void keys.refetch();
  }

  return (
    <>
      <section
        aria-labelledby="permissions-heading"
        className="grid gap-6 border-t border-border py-8 sm:grid-cols-[180px_1fr]"
      >
        <div>
          <p className="mb-2 font-mono text-xs text-muted-foreground">
            02 / PERMISSIONS
          </p>
          <h2 id="permissions-heading" className="text-lg font-medium">
            Check availability
          </h2>
        </div>
        <div className="space-y-5">
          {catalog.isPending ? (
            <p role="status" className="text-sm text-muted-foreground">
              Loading Google configuration…
            </p>
          ) : catalog.isError ? (
            <p role="alert" className="text-sm text-destructive">
              Unable to load Google: {catalog.error.message}
            </p>
          ) : (
            <>
              <div>
                <p className="font-medium">{entry?.name ?? "Google"}</p>
                <p className="mt-1 text-sm leading-6 text-muted-foreground">
                  {managedReady
                    ? "NyxID’s Google app is configured. Google will validate the app and callback when authorization opens."
                    : "NyxID-managed OAuth is unavailable. An administrator needs to configure the Google provider’s platform credentials and enable managed connections."}
                </p>
              </div>
              <div className="rounded-xl border border-border bg-card p-5">
                <p className="text-sm font-medium">Basic account binding</p>
                <p className="mt-1 text-sm leading-6 text-muted-foreground">
                  Identity permissions establish a Google connection. Drive,
                  Gmail, and Calendar access each need additional permission.
                </p>
                <p className="mt-3 break-words font-mono text-xs text-muted-foreground">
                  Managed scope allowlist:{" "}
                  {allowlist
                    ? allowlist.join(" · ") || "No scopes enabled"
                    : "Not reported by this backend"}
                </p>
              </div>
              <ul className="divide-y divide-border">
                {WORKSPACE_PERMISSIONS.map((permission) => {
                  const allowed = allowlist?.includes(permission.scope);
                  return (
                    <li
                      key={permission.scope}
                      className="flex flex-wrap items-center justify-between gap-3 py-3"
                    >
                      <div>
                        <p className="text-sm font-medium">{permission.name}</p>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {permission.detail}
                        </p>
                      </div>
                      <span
                        className={`rounded-md border px-2 py-1 text-xs ${allowed ? "border-emerald-500/30 text-emerald-500" : "border-border text-muted-foreground"}`}
                      >
                        {allowlist == null
                          ? "Check in consent setup"
                          : allowed
                            ? "Available to request"
                            : "Not enabled for managed app"}
                      </span>
                    </li>
                  );
                })}
              </ul>
            </>
          )}
          <Button
            variant="outline"
            onClick={refresh}
            isLoading={catalog.isFetching || keys.isFetching}
          >
            <RefreshCw />
            Refresh configuration & connections
          </Button>
        </div>
      </section>

      <section
        aria-labelledby="connect-heading"
        className="grid gap-6 border-t border-border py-8 sm:grid-cols-[180px_1fr]"
      >
        <div>
          <p className="mb-2 font-mono text-xs text-muted-foreground">
            03 / AUTHORIZE
          </p>
          <h2 id="connect-heading" className="text-lg font-medium">
            Bind Google
          </h2>
        </div>
        <div className="space-y-4">
          <p className="text-sm leading-6 text-muted-foreground">
            Open the connection dialog, keep{" "}
            <strong className="text-foreground">NyxID managed</strong> selected,
            then review the permissions and follow the Google authorization
            link. Each new connection creates a real service in your account.
          </p>
          <Button
            size="lg"
            disabled={!managedReady || dialogOpen}
            onClick={() => setDialogOpen(true)}
          >
            Connect Google through NyxID <ExternalLink />
          </Button>
          {attempt && (
            <div
              role="status"
              className="rounded-xl border border-border bg-card p-5"
            >
              <p className="text-sm font-medium">
                {watch.authorized
                  ? "Google authorization confirmed by NyxID"
                  : watch.status === "failed"
                    ? "Google authorization failed"
                    : aborted
                      ? "Dialog closed — refresh connections to check the result"
                      : watch.timedOut
                        ? "Waiting stopped — refresh connections or try again"
                        : "Waiting for Google authorization…"}
              </p>
              <p className="mt-2 text-sm leading-6 text-muted-foreground">
                {watch.authorized
                  ? "The saved credential became active for this attempt. Workspace access depends on the permissions you granted."
                  : (watch.errorMessage ??
                    "Completion is checked with NyxID’s API, even when the Google callback finishes on another origin.")}
              </p>
              <p className="mt-3 break-all font-mono text-xs text-muted-foreground">
                Connection: {attempt.keyId}
              </p>
            </div>
          )}
        </div>
      </section>

      <section
        aria-labelledby="connections-heading"
        className="grid gap-6 border-t border-border py-8 sm:grid-cols-[180px_1fr]"
      >
        <div>
          <p className="mb-2 font-mono text-xs text-muted-foreground">
            04 / VERIFY
          </p>
          <h2 id="connections-heading" className="text-lg font-medium">
            Saved connections
          </h2>
        </div>
        <div className="space-y-3">
          {keys.isError ? (
            <p role="alert" className="text-sm text-destructive">
              Unable to load connections: {keys.error.message}
            </p>
          ) : keys.isPending ? (
            <p className="text-sm text-muted-foreground">
              Loading connections…
            </p>
          ) : googleKeys.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No saved Google connections yet.
            </p>
          ) : (
            googleKeys.map((key) => (
              <a
                key={key.id}
                href={`/keys/${encodeURIComponent(key.id)}`}
                className="block rounded-xl border border-border p-4 transition-colors hover:bg-card"
              >
                <div className="flex flex-wrap justify-between gap-2">
                  <span className="text-sm font-medium">
                    {key.label || key.slug}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    Service: {key.is_active ? "Enabled" : "Disabled"} ·
                    Credential: {key.status}
                  </span>
                </div>
                <p className="mt-2 break-all font-mono text-xs text-muted-foreground">
                  {key.slug}
                </p>
                {key.last_authorized_at && (
                  <p className="mt-2 text-xs text-muted-foreground">
                    Last authorized:{" "}
                    {new Date(key.last_authorized_at).toLocaleString()}
                  </p>
                )}
              </a>
            ))
          )}
          <p className="text-xs leading-5 text-muted-foreground">
            An existing connection is listed separately from the result of your
            current authorization attempt.
          </p>
        </div>
      </section>

      {dialogOpen && (
        <AddKeyDialog
          open={dialogOpen}
          onOpenChange={setDialogOpen}
          prefillSlug={GOOGLE_CATALOG_SLUG}
          prefillIncludeAllCatalog
          onAuthorizationPending={(pending) => {
            setAborted(false);
            setAttempt({ ...pending, deadlineAt: Date.now() + 10 * 60 * 1000 });
          }}
          onAuthorizationAborted={(attemptId) => {
            if (attempt?.attemptId === attemptId) setAborted(true);
          }}
          onSuccess={() => {
            void keys.refetch();
          }}
        />
      )}
    </>
  );
}
