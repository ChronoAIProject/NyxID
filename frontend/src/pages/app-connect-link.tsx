import { useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { AppConnectChecklistItem } from "@/components/connect/app-connect-checklist";
import { AppConnectShell } from "@/components/connect/app-connect-shell";
import {
  ConnectShell,
  CredentialForm,
  DeviceCodePanel,
  OAuthSetupForm,
} from "@/components/connect/connection-panels";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useAppConnectAction,
  useAppConnectChild,
  useAppConnectLink,
} from "@/hooks/use-app-connect-links";
import { useCompleteConnectLink } from "@/hooks/use-connect-links";
import {
  appConnectCapabilityStorageKey,
  requirementsReady,
} from "@/lib/app-connect-link";
import {
  connectLinkErrorMessage,
  connectLinkNeedsSetupForm,
} from "@/lib/connect-link-page";
import type { AppConnectChild } from "@/schemas/app-connect-links";
import type { CompleteConnectLinkInput } from "@/schemas/connect-links";
import { useAuthStore } from "@/stores/auth-store";

export function AppConnectLinkPage() {
  const { linkId } = useParams({ strict: false }) as { linkId: string };
  const navigate = useNavigate();
  const { user, isAuthenticated, isLoading } = useAuthStore();
  useEffect(() => {
    if (!isLoading && !isAuthenticated) {
      const returnTo = new URL(window.location.href);
      const capability = new URLSearchParams(returnTo.hash.slice(1)).get("t");
      if (capability)
        sessionStorage.setItem(
          appConnectCapabilityStorageKey(linkId),
          capability,
        );
      returnTo.hash = "";
      void navigate({
        to: "/login",
        search: { return_to: returnTo.toString() },
      });
    }
  }, [isAuthenticated, isLoading, linkId, navigate]);
  if (isLoading || !isAuthenticated || !user) {
    return (
      <ConnectShell>
        <Skeleton className="h-72 w-full" />
      </ConnectShell>
    );
  }
  return (
    <RepairSession
      key={`${user.id}:${linkId}`}
      subject={user.id}
      linkId={linkId}
    />
  );
}

function RepairSession({
  subject,
  linkId,
}: {
  readonly subject: string;
  readonly linkId: string;
}) {
  const [capability] = useState(
    () =>
      new URLSearchParams(window.location.hash.slice(1)).get("t") ??
      sessionStorage.getItem(appConnectCapabilityStorageKey(linkId)),
  );
  const session = useAppConnectLink(linkId, subject, true, capability);
  const action = useAppConnectAction(linkId, subject);
  const child = useAppConnectChild(linkId);
  const complete = useCompleteConnectLink();
  const [setup, setSetup] = useState<{
    requirementId: string;
    child: AppConnectChild;
  } | null>(null);
  const [device, setDevice] = useState<{
    code: string;
    url: string;
    state: string;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const lastClick = useRef(-Infinity);
  const [now, setNow] = useState(Date.now);

  useEffect(() => {
    if (session.data && capability) {
      const clean = new URL(window.location.href);
      clean.hash = "";
      window.history.replaceState(window.history.state, "", clean);
      sessionStorage.removeItem(appConnectCapabilityStorageKey(linkId));
    }
  }, [session.data, capability, linkId]);

  // Time changes presentation only. Reads and provider checks remain explicit.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  async function run(operation: () => Promise<void>) {
    if (busyRef.current || Date.now() - lastClick.current < 750) return;
    lastClick.current = Date.now();
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      await operation();
    } catch (cause) {
      setError(connectLinkErrorMessage(cause));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  async function submitChild(
    target: AppConnectChild,
    values?: CompleteConnectLinkInput,
  ) {
    const result = await complete.mutateAsync({ token: target.token, values });
    if (result.status === "oauth_required" && result.authorization_url) {
      window.location.assign(result.authorization_url);
    } else if (result.status === "device_code_required") {
      setDevice((previous) => ({
        code: result.device_user_code ?? previous?.code ?? "",
        url: result.device_verification_uri ?? previous?.url ?? "",
        state: result.device_state ?? previous?.state ?? "",
      }));
    } else if (result.status === "completed") {
      setSetup(null);
      setDevice(null);
    }
    // Child completion is reconciled by a read; checking the provider is a
    // separate explicit action, including after an OAuth return.
    await session.refetch();
  }

  async function connect(
    requirementId: string,
    slug: string,
    reauthorize: boolean,
  ) {
    const target = await child.mutateAsync({
      requirementId,
      slug,
      reauthorize,
    });
    setSetup({ requirementId, child: target });
    setDevice(null);
    await session.refetch();
    if (reauthorize || !connectLinkNeedsSetupForm(target))
      await submitChild(target);
  }

  async function post(path: string, body?: unknown) {
    const result = await action.mutateAsync({ path, body });
    if (path === "ready" || path === "cancel") {
      if (result.consent_url) window.location.assign(result.consent_url);
      else if (result.callback_url) window.location.assign(result.callback_url);
    } else if (path.endsWith("/select")) {
      setSetup(null);
      setDevice(null);
    }
  }

  const link = session.data;
  if (session.error || !link) {
    return (
      <ConnectShell>
        {session.error ? (
          <ErrorBanner message={connectLinkErrorMessage(session.error)} />
        ) : (
          <Skeleton className="h-72 w-full" />
        )}
      </ConnectShell>
    );
  }
  const terminal =
    link.status !== "in_progress" && link.status !== "ready_for_consent";
  const expired = new Date(link.expires_at).getTime() <= now;
  const unsatisfiable =
    link.origin === "authorize" &&
    link.items.some(
      (item) =>
        !item.optional &&
        item.readiness === "unsatisfiable" &&
        item.reason_code !== "slug_shadowed",
    );
  return (
    <AppConnectShell
      name={link.client_name}
      blurb={link.handoff_blurb}
      destination={link.destination}
    >
      {error && <ErrorBanner message={error} />}
      {link.status === "ready_for_consent" ? (
        <Card>
          <CardContent className="space-y-3 p-5">
            <p className="text-[12px]">
              Your connections are ready. Review the app’s access to continue.
            </p>
            <Button
              variant="primary"
              disabled={busy || expired}
              onClick={() => void run(() => post("ready"))}
            >
              Review access
            </Button>
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() => void run(() => post("cancel"))}
            >
              Not now
            </Button>
          </CardContent>
        </Card>
      ) : terminal ? (
        <Card>
          <CardContent className="space-y-3 p-5">
            <h2 className="text-[15px] font-semibold">
              {link.status === "completed"
                ? "Connections ready"
                : link.status === "cancelled"
                  ? "Connection setup cancelled"
                  : link.status === "expired"
                    ? "Connection request expired"
                    : "Connection setup failed"}
            </h2>
            <p className="text-[12px] text-muted-foreground">
              {link.origin === "authorize" && link.status === "expired"
                ? "Restart sign-in from the app. This expired request cannot issue an authorization code."
                : link.grant_update_required
                  ? "The app will ask for your permission to use the new connections when you return."
                  : "Return to the app to continue. Checking connections does not grant the app additional access."}
            </p>
            {link.callback_url && (
              <Button
                variant="primary"
                onClick={() => window.location.assign(link.callback_url!)}
              >
                Return to {link.client_name}
              </Button>
            )}
          </CardContent>
        </Card>
      ) : (
        <>
          <p className="text-[12px] text-muted-foreground">
            Choose the connected accounts to use. Connection checks contact the
            provider; provider rate limits apply.
          </p>
          {expired && (
            <ErrorBanner message="This request has expired. Return to the app and start a new request." />
          )}
          {link.items.map((item) => (
            <AppConnectChecklistItem
              key={item.requirement_id}
              item={item}
              now={now}
              pending={busy || expired}
              onConnect={(slug, reauthorize) =>
                void run(() => connect(item.requirement_id, slug, reauthorize))
              }
              onSelect={(id) =>
                void run(() =>
                  post(
                    `items/${encodeURIComponent(item.requirement_id)}/select`,
                    { user_service_id: id },
                  ),
                )
              }
              onValidate={() =>
                void run(() =>
                  post(
                    `items/${encodeURIComponent(item.requirement_id)}/validate`,
                  ),
                )
              }
            >
              {setup?.requirementId === item.requirement_id &&
                (device ? (
                  <DeviceCodePanel
                    code={device.code}
                    url={device.url}
                    pending={busy || expired}
                    onCheck={() =>
                      void run(() =>
                        submitChild(setup.child, {
                          device_state: device.state,
                        }),
                      )
                    }
                  />
                ) : setup.child.connect_method === "api_key" ? (
                  <CredentialForm
                    preview={setup.child}
                    pending={busy || expired}
                    onSubmit={(values) =>
                      void run(() => submitChild(setup.child, values))
                    }
                  />
                ) : (
                  connectLinkNeedsSetupForm(setup.child) && (
                    <OAuthSetupForm
                      preview={setup.child}
                      pending={busy || expired}
                      onSubmit={(values) =>
                        void run(() => submitChild(setup.child, values))
                      }
                    />
                  )
                ))}
            </AppConnectChecklistItem>
          ))}
          <div className="flex justify-end gap-2">
            {link.can_try_later && (
              <Button
                disabled={busy || expired}
                onClick={() =>
                  void run(() => post("cancel", { try_later: true }))
                }
              >
                Try later
              </Button>
            )}
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  if (expired) {
                    const result = await session.refetch();
                    if (result.data?.callback_url)
                      window.location.assign(result.data.callback_url);
                  } else {
                    await post("cancel");
                  }
                })
              }
            >
              Not now
            </Button>
            <Button
              variant="primary"
              disabled={
                busy ||
                expired ||
                (!unsatisfiable && !requirementsReady(link, now))
              }
              onClick={() => void run(() => post("ready"))}
            >
              {unsatisfiable ? `Return to ${link.client_name}` : "Continue"}
            </Button>
          </div>
        </>
      )}
    </AppConnectShell>
  );
}
