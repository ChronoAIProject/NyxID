import { useCatalogEntry } from "@/hooks/use-keys";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ConnectionArc } from "@/components/shared/connection-arc";
import { ServiceIcon } from "@/components/service-icon";
import { useApplyTheme } from "@/hooks/use-theme";
import { CredentialBindingChoice } from "@/components/shared/credential-binding-choice";
import { useEffect, useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useNavigate, useParams } from "@tanstack/react-router";
import { CheckCircle2, ExternalLink, ArrowRight, XCircle } from "lucide-react";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ApiError } from "@/lib/api-client";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  connectLinkStorageKey,
  useCancelHostedConnectLink,
  useCompleteConnectLink,
  useConnectLinkStatus,
  usePreviewConnectLink,
} from "@/hooks/use-connect-links";
import {
  connectLinkErrorMessage,
  connectLinkNeedsOAuthCredentials,
  connectLinkNeedsSetupForm,
  connectLinkShowsEndpointUrl,
  connectLinkProviderError,
} from "@/lib/connect-link-page";
import {
  connectCredentialFormSchema,
  connectOAuthFormSchema,
  type CompleteConnectLinkInput,
  type ConnectCredentialForm,
  type ConnectLinkPreview,
  type ConnectOAuthForm,
  validateConnectCredentialForm,
  validateConnectOAuthForm,
} from "@/schemas/connect-links";
import { useAuthStore } from "@/stores/auth-store";
import { cn } from "@/lib/utils";

const CLICK_THROTTLE_MS = 750;

interface DeviceChallenge {
  readonly code: string;
  readonly url: string;
  readonly state: string;
  readonly interval: number;
  readonly status: string;
}

export function ConnectLinkContent({
  token,
  embedded = false,
  redirectOnTerminal = true,
}: {
  readonly token: string;
  /** Render inside an existing dialog instead of the standalone hosted page. */
  readonly embedded?: boolean;
  /** Hosted pages return to the requesting app; chat modals stay in place. */
  readonly redirectOnTerminal?: boolean;
}) {
  const navigate = useNavigate();
  const { isAuthenticated, isLoading, user } = useAuthStore();
  const [deviceChallenge, setDeviceChallenge] =
    useState<DeviceChallenge | null>(null);
  const [oauthPopupOpen, setOauthPopupOpen] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const lastClickAtRef = useRef(0);
  const previewedTokenRef = useRef<string | null>(null);
  const oauthTokenRef = useRef<{ id: string; token: string } | null>(null);
  const oauthWindowRef = useRef<Window | null>(null);
  const mountedRef = useRef(true);
  const preview = usePreviewConnectLink();
  const previewConnect = preview.mutateAsync;
  const [oauthLinkId, setOauthLinkId] = useState<string | null>(null);
  const oauthStatus = useConnectLinkStatus(
    oauthLinkId ?? "",
    embedded && oauthLinkId !== null,
  );
  const [platformChoice, setPlatformChoice] = useState<boolean | null>(null);
  const { data: catalog } = useCatalogEntry(
    isAuthenticated ? preview.data?.service_slug : undefined,
  );
  const platformAvailable = Boolean(
    catalog?.platform_key?.available && !preview.data?.scopes.length,
  );
  const usePlatformKey =
    platformAvailable &&
    (platformChoice ?? preview.data?.use_platform_key ?? true);
  const complete = useCompleteConnectLink();
  const completeConnect = complete.mutateAsync;
  const cancel = useCancelHostedConnectLink();
  const actionPending =
    preview.isPending || complete.isPending || cancel.isPending;

  useEffect(() => {
    if (isLoading || isAuthenticated) return;
    const returnTo = `${window.location.origin}/connect/${encodeURIComponent(token)}`;
    void navigate({ to: "/login", search: { return_to: returnTo } });
  }, [isAuthenticated, isLoading, navigate, token]);

  useEffect(() => {
    if (isLoading || !isAuthenticated || previewedTokenRef.current === token)
      return;
    let active = true;
    queueMicrotask(() => {
      if (!active || previewedTokenRef.current === token) return;
      previewedTokenRef.current = token;
      void previewConnect(token).catch((error) => {
        if (active) setSubmitError(connectLinkErrorMessage(error));
      });
    });
    return () => {
      active = false;
    };
  }, [isAuthenticated, isLoading, previewConnect, token]);

  useEffect(() => {
    if (!embedded) return;
    const handleMessage = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      const current = oauthTokenRef.current;
      if (!current || event.source !== oauthWindowRef.current) return;
      if (!event.data || typeof event.data !== "object") return;
      const data = event.data as {
        type?: unknown;
        connect_link_id?: unknown;
        status?: unknown;
        token?: unknown;
      };
      if (data.connect_link_id !== current.id) return;
      if (
        data.type === "nyxid-connect-token-request" &&
        typeof data.token !== "string" &&
        event.source
      ) {
        event.source.postMessage(
          {
            type: "nyxid-connect-token",
            connect_link_id: current.id,
            token: current.token,
          },
          event.origin,
        );
        return;
      }
      if (data.type !== "nyxid-connect-finished") return;
      setOauthPopupOpen(false);
      oauthTokenRef.current = null;
      oauthWindowRef.current?.close();
      oauthWindowRef.current = null;
      void previewConnect(token).catch((error) => {
        setSubmitError(connectLinkErrorMessage(error));
      });
    };
    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, [embedded, previewConnect, token]);

  useEffect(() => {
    if (
      !embedded ||
      !oauthLinkId ||
      !oauthStatus.data ||
      oauthStatus.data.status === "pending"
    ) {
      return;
    }
    let active = true;
    const timer = window.setTimeout(() => {
      if (!active) return;
      setOauthPopupOpen(false);
      setOauthLinkId(null);
      oauthTokenRef.current = null;
      oauthWindowRef.current?.close();
      oauthWindowRef.current = null;
      void previewConnect(token).catch((error) => {
        setSubmitError(connectLinkErrorMessage(error));
      });
    }, 0);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [embedded, oauthLinkId, oauthStatus.data, previewConnect, token]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      oauthWindowRef.current?.close();
      oauthWindowRef.current = null;
      oauthTokenRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!oauthPopupOpen) return;
    const timer = window.setInterval(() => {
      if (!oauthWindowRef.current?.closed) return;
      oauthWindowRef.current = null;
      oauthTokenRef.current = null;
      setOauthPopupOpen(false);
      setSubmitError("Authorization was cancelled. You can try again.");
    }, 500);
    return () => window.clearInterval(timer);
  }, [oauthPopupOpen]);

  useEffect(() => {
    const callback =
      complete.data?.status === "completed"
        ? complete.data.callback_url
        : (cancel.data?.callback_url ?? preview.data?.callback_url);
    if (!callback || !redirectOnTerminal) return;
    const timer = window.setTimeout(
      () => window.location.assign(callback),
      1_500,
    );
    return () => window.clearTimeout(timer);
  }, [cancel.data, complete.data, preview.data, redirectOnTerminal]);

  function withinCooldown(): boolean {
    const now = Date.now();
    if (now - lastClickAtRef.current < CLICK_THROTTLE_MS) return true;
    lastClickAtRef.current = now;
    return false;
  }

  async function handlePreview() {
    if (actionPending || withinCooldown()) return;
    setSubmitError(null);
    try {
      await preview.mutateAsync(token);
    } catch (error) {
      setSubmitError(connectLinkErrorMessage(error));
    }
  }

  async function submitCompletion(values?: CompleteConnectLinkInput) {
    setSubmitError(null);
    // Reserve the popup synchronously while the submit event still has user
    // activation. The provider URL is assigned after the completion request
    // returns; opening it only after `await` is blocked by Safari and strict
    // popup policies.
    if (
      embedded &&
      preview.data?.connect_method === "oauth" &&
      !oauthWindowRef.current
    ) {
      oauthWindowRef.current = window.open(
        "about:blank",
        "_blank",
        "popup,width=560,height=720,resizable=yes,scrollbars=yes",
      );
    }
    try {
      const selected = { use_platform_key: usePlatformKey };
      const result = await complete.mutateAsync({
        token,
        values: usePlatformKey ? selected : { ...values, ...selected },
      });
      if (!mountedRef.current) return;
      if (result.status === "oauth_required" && result.authorization_url) {
        if (embedded) {
          if (oauthWindowRef.current?.closed) {
            oauthWindowRef.current = null;
          }
          const popup =
            oauthWindowRef.current ??
            window.open(
              "about:blank",
              "_blank",
              "popup,width=560,height=720,resizable=yes,scrollbars=yes",
            );
          if (!popup) {
            setSubmitError(
              "Your browser blocked the authorization popup. Allow popups for NyxID and try again.",
            );
            return;
          }
          // The popup copied session storage when it opened, before the link
          // id existed. Store its recovery token there before leaving our origin.
          popup.sessionStorage.setItem(connectLinkStorageKey(result.id), token);
          oauthTokenRef.current = { id: result.id, token };
          oauthWindowRef.current = popup;
          setOauthLinkId(result.id);
          setOauthPopupOpen(true);
          popup.location.assign(result.authorization_url);
          return;
        }
        sessionStorage.setItem(connectLinkStorageKey(result.id), token);
        window.location.assign(result.authorization_url);
        return;
      }
      if (embedded) {
        oauthWindowRef.current?.close();
        oauthWindowRef.current = null;
      }
      if (result.status === "device_code_required") {
        setDeviceChallenge((current) => ({
          code: result.device_user_code ?? current?.code ?? "",
          url: result.device_verification_uri ?? current?.url ?? "",
          state: result.device_state ?? current?.state ?? "",
          interval: Math.max(
            1,
            result.device_interval ?? current?.interval ?? 5,
          ),
          status: result.device_status ?? current?.status ?? "pending",
        }));
        return;
      }
      setDeviceChallenge(null);
    } catch (error) {
      if (!mountedRef.current) return;
      if (embedded) {
        oauthWindowRef.current?.close();
        oauthWindowRef.current = null;
      }
      setSubmitError(connectLinkErrorMessage(error));
      await refreshPreviewAfterTerminalError();
    }
  }

  async function refreshPreviewAfterTerminalError() {
    try {
      await preview.mutateAsync(token);
    } catch {
      // Preserve the original action error when the follow-up preview also fails.
    }
  }

  async function handleCancel() {
    if (actionPending || withinCooldown()) return;
    setSubmitError(null);
    try {
      await cancel.mutateAsync(token);
    } catch (error) {
      setSubmitError(connectLinkErrorMessage(error));
      await refreshPreviewAfterTerminalError();
    }
  }

  function handleCredentialSubmit(values: ConnectCredentialForm) {
    if (actionPending || withinCooldown()) return;
    void submitCompletion(values);
  }

  function handleOAuthSubmit(values: ConnectOAuthForm) {
    if (actionPending || withinCooldown()) return;
    void submitCompletion(values);
  }

  useEffect(() => {
    if (!deviceChallenge?.state || cancel.data?.status === "cancelled") return;
    let cancelled = false;
    let failures = 0;
    let timer: number | undefined;
    const state = deviceChallenge.state;
    const schedule = (seconds: number) => {
      timer = window.setTimeout(
        () => {
          if (cancelled) return;
          void (async () => {
            try {
              const result = await completeConnect({
                token,
                values: {
                  use_platform_key: usePlatformKey,
                  device_state: state,
                },
              });
              if (cancelled) return;
              if (result.status !== "device_code_required") {
                setDeviceChallenge(null);
                return;
              }
              failures = 0;
              const nextInterval = Math.max(
                1,
                result.device_interval ?? deviceChallenge.interval,
              );
              setSubmitError(null);
              setDeviceChallenge((current) =>
                current?.state === state
                  ? {
                      ...current,
                      interval: nextInterval,
                      status: result.device_status ?? current.status,
                    }
                  : current,
              );
              schedule(nextInterval);
            } catch (error) {
              if (cancelled) return;
              failures += 1;
              const terminalError =
                error instanceof ApiError &&
                error.status >= 400 &&
                error.status < 500 &&
                error.status !== 429;
              if (terminalError || failures >= 3) {
                setSubmitError(connectLinkErrorMessage(error));
                return;
              }
              schedule(
                deviceChallenge.interval +
                  (error instanceof ApiError && error.status === 429 ? 5 : 0),
              );
            }
          })();
        },
        Math.max(1, seconds) * 1_000,
      );
    };
    schedule(deviceChallenge.interval);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [
    cancel.data?.status,
    completeConnect,
    deviceChallenge?.interval,
    deviceChallenge?.state,
    token,
    usePlatformKey,
  ]);

  if (isLoading || !isAuthenticated) {
    return (
      <ConnectShell embedded={embedded}>
        <Skeleton className="h-72 w-full" />
      </ConnectShell>
    );
  }

  const terminal =
    complete.data?.status === "completed"
      ? {
          status: "completed" as const,
          callbackUrl: complete.data.callback_url,
        }
      : cancel.data && cancel.data.status !== "pending"
        ? { status: cancel.data.status, callbackUrl: cancel.data.callback_url }
        : preview.data && preview.data.status !== "pending"
          ? {
              status: preview.data.status,
              callbackUrl: preview.data.callback_url,
            }
          : null;
  return (
    <ConnectShell embedded={embedded}>
      {terminal ? (
        <TerminalPanel
          status={terminal.status}
          serviceName={preview.data?.service_name}
          callbackUrl={embedded ? null : (terminal.callbackUrl ?? null)}
        />
      ) : (
        <Card className="connect-link-card border-border/70">
          <CardContent className="space-y-6 p-5 sm:p-8">
            {!preview.data ? (
              <div className="space-y-5 py-4 text-center">
                <Skeleton className="mx-auto h-20 w-52" />
                <Skeleton className="mx-auto h-7 w-4/5" />
                {submitError ? <ErrorBanner message={submitError} /> : null}
                {submitError ? (
                  <Button
                    type="button"
                    variant="primary"
                    disabled={actionPending}
                    isLoading={preview.isPending}
                    onClick={() => void handlePreview()}
                  >
                    Try again
                  </Button>
                ) : null}
              </div>
            ) : (
              <>
                <div className="connection-card-heading space-y-5 text-center">
                  <div
                    className="flex items-start justify-center"
                    role="img"
                    aria-label={`NyxID connects to ${preview.data.service_name}`}
                  >
                    <div className="flex w-20 flex-col items-center gap-2.5">
                      <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
                        <NyxidIcon className="size-9" alt="" />
                      </div>
                      <span className="text-xs font-medium">NyxID</span>
                    </div>
                    <ConnectionArc />
                    <div className="flex w-20 flex-col items-center gap-2.5">
                      <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
                        {catalog?.icon_url ? (
                          <img
                            src={catalog.icon_url}
                            alt=""
                            className="size-9 object-contain"
                          />
                        ) : (
                          <ServiceIcon
                            slug={preview.data.service_slug}
                            size="xl"
                            className="text-foreground"
                          />
                        )}
                      </div>
                      <span className="text-xs font-medium">
                        {preview.data.service_name}
                      </span>
                    </div>
                  </div>
                  <div className="space-y-2">
                    <h1 className="text-xl font-semibold leading-tight text-foreground sm:text-2xl">
                      NyxID wants to connect to your {preview.data.service_name}
                    </h1>
                    <p className="text-sm text-muted-foreground">
                      Add your connection details and approve the request.
                    </p>
                  </div>
                </div>
                {submitError ? <ErrorBanner message={submitError} /> : null}
                <RequestDetails preview={preview.data} />
                {platformAvailable &&
                  catalog?.platform_key &&
                  preview.data.status === "pending" && (
                    <CredentialBindingChoice
                      value={usePlatformKey}
                      onChange={setPlatformChoice}
                      platformPrice={catalog.platform_key.pricing}
                      byokPrice={catalog.byok_pricing}
                      legacyBillable={catalog.billing?.platform_billable}
                      resaleBillable={catalog.billing?.resale_billable}
                      disabled={actionPending || !!deviceChallenge}
                    />
                  )}
                {preview.data.status !== "pending" ? (
                  <ErrorBanner
                    message={`This connection request is ${preview.data.status}.`}
                  />
                ) : null}
                {oauthPopupOpen ? (
                  <p
                    className="rounded-lg border border-border/50 bg-muted/30 px-3 py-2 text-xs text-muted-foreground"
                    role="status"
                  >
                    Finish authorization in the provider window, then return
                    here.
                  </p>
                ) : null}
                {preview.data.status === "pending" &&
                !deviceChallenge &&
                !oauthPopupOpen ? (
                  !usePlatformKey &&
                  preview.data.connect_method === "api_key" ? (
                    <CredentialForm
                      preview={preview.data}
                      pending={actionPending}
                      onSubmit={handleCredentialSubmit}
                    />
                  ) : !usePlatformKey &&
                    connectLinkNeedsSetupForm(preview.data) ? (
                    <OAuthSetupForm
                      preview={preview.data}
                      pending={actionPending}
                      onSubmit={handleOAuthSubmit}
                    />
                  ) : (
                    <Button
                      type="button"
                      variant="primary"
                      className="w-full"
                      disabled={actionPending}
                      isLoading={complete.isPending}
                      onClick={() => {
                        if (!withinCooldown()) void submitCompletion();
                      }}
                    >
                      Approve connection{" "}
                      <ButtonIcon variant="primary">
                        <ArrowRight />
                      </ButtonIcon>
                    </Button>
                  )
                ) : null}
                {deviceChallenge ? (
                  <DeviceCodePanel
                    code={deviceChallenge.code}
                    url={deviceChallenge.url}
                    interval={deviceChallenge.interval}
                    status={deviceChallenge.status}
                  />
                ) : null}
                {deviceChallenge && submitError ? (
                  <Button
                    type="button"
                    variant="outline"
                    disabled={actionPending}
                    onClick={() => void submitCompletion()}
                  >
                    Get a new code
                  </Button>
                ) : null}
                {preview.data.status === "pending" ? (
                  <div className="flex justify-center">
                    <Button
                      type="button"
                      variant="ghost"
                      disabled={actionPending}
                      isLoading={cancel.isPending}
                      onClick={() => void handleCancel()}
                    >
                      Decline
                    </Button>
                  </div>
                ) : null}
              </>
            )}
          </CardContent>
          {user?.email ? (
            <div className="border-t border-border px-5 py-3 text-center text-xs text-muted-foreground">
              Signed in as <span className="text-foreground">{user.email}</span>
            </div>
          ) : null}
        </Card>
      )}
    </ConnectShell>
  );
}

export function ConnectLinkReturnPage() {
  const { linkId } = useParams({ strict: false }) as { linkId: string };
  const navigate = useNavigate();
  const { isAuthenticated, isLoading } = useAuthStore();
  const status = useConnectLinkStatus(linkId, isAuthenticated && !isLoading);
  const complete = useCompleteConnectLink();
  const [recoveryError, setRecoveryError] = useState<string | null>(null);
  const recoveryAttemptedRef = useRef(false);
  const openerRequestSentRef = useRef(false);
  const providerError = connectLinkProviderError(window.location.search);

  useEffect(() => {
    if (isLoading || isAuthenticated) return;
    const returnTo = `${window.location.origin}/connect/return/${linkId}`;
    void navigate({ to: "/login", search: { return_to: returnTo } });
  }, [isAuthenticated, isLoading, linkId, navigate]);

  useEffect(() => {
    if (providerError || !window.opener) return;
    const handleMessage = (event: MessageEvent) => {
      if (
        event.origin !== window.location.origin ||
        event.source !== window.opener
      )
        return;
      if (!event.data || typeof event.data !== "object") return;
      const data = event.data as {
        type?: unknown;
        connect_link_id?: unknown;
        token?: unknown;
      };
      if (
        data.type !== "nyxid-connect-token" ||
        data.connect_link_id !== linkId ||
        typeof data.token !== "string" ||
        recoveryAttemptedRef.current
      ) {
        return;
      }
      recoveryAttemptedRef.current = true;
      void complete
        .mutateAsync({ token: data.token })
        .then((result) => {
          if (result.status === "completed") void status.refetch();
        })
        .catch((error) => setRecoveryError(connectLinkErrorMessage(error)));
    };
    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, [complete, linkId, providerError, status]);

  useEffect(() => {
    if (
      providerError ||
      status.data?.status !== "pending" ||
      recoveryAttemptedRef.current
    ) {
      return;
    }
    const token = sessionStorage.getItem(connectLinkStorageKey(linkId));
    if (token) {
      recoveryAttemptedRef.current = true;
      void complete
        .mutateAsync({ token })
        .then((result) => {
          if (result.status === "completed") void status.refetch();
        })
        .catch((error) => setRecoveryError(connectLinkErrorMessage(error)));
      return;
    }
    if (window.opener && !openerRequestSentRef.current) {
      openerRequestSentRef.current = true;
      window.opener.postMessage(
        {
          type: "nyxid-connect-token-request",
          connect_link_id: linkId,
        },
        window.location.origin,
      );
    }
  }, [complete, linkId, providerError, status]);

  useEffect(() => {
    if (
      status.data?.status !== "completed" &&
      status.data?.status !== "cancelled" &&
      status.data?.status !== "expired"
    )
      return;
    sessionStorage.removeItem(connectLinkStorageKey(linkId));
    if (window.opener) {
      window.opener.postMessage(
        {
          type: "nyxid-connect-finished",
          connect_link_id: linkId,
          status: status.data.status,
        },
        window.location.origin,
      );
    }
    const callback = status.data.callback_url;
    if (!callback) return;
    const timer = window.setTimeout(
      () => window.location.assign(callback),
      1_500,
    );
    return () => window.clearTimeout(timer);
  }, [linkId, status.data]);

  if (isLoading || !isAuthenticated || status.isLoading) {
    return (
      <ConnectShell>
        <Skeleton className="h-60 w-full" />
      </ConnectShell>
    );
  }
  if (status.error) {
    return (
      <ConnectShell>
        <ErrorBanner message={connectLinkErrorMessage(status.error)} />
      </ConnectShell>
    );
  }
  if (
    status.data?.status === "completed" ||
    status.data?.status === "cancelled" ||
    status.data?.status === "expired"
  ) {
    return (
      <ConnectShell>
        <TerminalPanel
          status={status.data.status}
          serviceName={status.data.service_name}
          callbackUrl={status.data.callback_url ?? null}
        />
      </ConnectShell>
    );
  }
  if (providerError) {
    const token = sessionStorage.getItem(connectLinkStorageKey(linkId));
    return (
      <ConnectShell>
        <ErrorBanner message={providerError} />
        {token ? (
          <div className="flex justify-end">
            <Button
              type="button"
              variant="primary"
              onClick={() =>
                window.location.assign(`/connect/${encodeURIComponent(token)}`)
              }
            >
              Try again
            </Button>
          </div>
        ) : null}
      </ConnectShell>
    );
  }
  return (
    <ConnectShell>
      {recoveryError ? <ErrorBanner message={recoveryError} /> : null}
      <Card className="border-border/50">
        <CardContent className="p-5 text-center text-[12px] text-muted-foreground">
          Finishing the connection...
        </CardContent>
      </Card>
    </ConnectShell>
  );
}

function CredentialForm({
  preview,
  pending,
  onSubmit,
}: {
  readonly preview: ConnectLinkPreview;
  readonly pending: boolean;
  readonly onSubmit: (values: ConnectCredentialForm) => void;
}) {
  const [formError, setFormError] = useState<string | null>(null);
  const form = useAppForm<ConnectCredentialForm>({
    resolver: zodResolver(connectCredentialFormSchema),
    defaultValues: {
      credential: "",
      endpoint_url: preview.endpoint_url ?? "",
      oauth_client_id: "",
      oauth_client_secret: "",
    },
  });
  const submit = form.handleSubmit((values) => {
    const error = validateConnectCredentialForm(
      values,
      preview.requires_gateway_url,
    );
    setFormError(error);
    if (!error) onSubmit(values);
  });
  const credential = form.watch("credential").trim();
  const endpointUrl = form.watch("endpoint_url").trim();
  const submitDisabled =
    pending ||
    credential.length === 0 ||
    (preview.requires_gateway_url && endpointUrl.length === 0);
  const credentialLabel =
    preview.auth_key_name.trim() ||
    (preview.requires_gateway_url
      ? "Gateway bearer token"
      : "API key or token");
  const credentialPlaceholder = preview.requires_gateway_url
    ? `Paste bearer token for ${preview.service_name}`
    : `Paste API key or token for ${preview.service_name}`;

  return (
    <Form {...form}>
      <form className="space-y-4" onSubmit={(event) => void submit(event)}>
        <FormField
          control={form.control}
          name="credential"
          render={({ field }) => (
            <FormItem>
              <FormLabel>{credentialLabel}</FormLabel>
              <FormControl>
                <Input
                  type="password"
                  autoComplete="off"
                  placeholder={credentialPlaceholder}
                  {...field}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        {connectLinkShowsEndpointUrl(preview) ? (
          <FormField
            control={form.control}
            name="endpoint_url"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Service URL</FormLabel>
                <FormControl>
                  <Input type="url" placeholder="https://" {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        ) : null}
        {preview.api_key_instructions ? (
          <p className="text-xs text-muted-foreground">
            {preview.api_key_instructions}
          </p>
        ) : null}
        {formError ? <ErrorBanner message={formError} /> : null}
        <div>
          <Button
            className="w-full"
            type="submit"
            variant="primary"
            disabled={submitDisabled}
            isLoading={pending}
          >
            Approve &amp; connect{" "}
            <ButtonIcon variant="primary">
              <ArrowRight />
            </ButtonIcon>
          </Button>
        </div>
      </form>
    </Form>
  );
}

function OAuthSetupForm({
  preview,
  pending,
  onSubmit,
}: {
  readonly preview: ConnectLinkPreview;
  readonly pending: boolean;
  readonly onSubmit: (values: ConnectOAuthForm) => void;
}) {
  const [formError, setFormError] = useState<string | null>(null);
  const requiresClientCredentials = connectLinkNeedsOAuthCredentials(preview);
  const form = useAppForm<ConnectOAuthForm>({
    resolver: zodResolver(connectOAuthFormSchema),
    defaultValues: {
      endpoint_url: preview.endpoint_url ?? "",
      oauth_client_id: "",
      oauth_client_secret: "",
    },
  });
  const submit = form.handleSubmit((values) => {
    const error = validateConnectOAuthForm(
      values,
      preview.requires_gateway_url,
      requiresClientCredentials,
    );
    setFormError(error);
    if (!error) onSubmit(values);
  });
  const endpointUrl = form.watch("endpoint_url").trim();
  const clientId = form.watch("oauth_client_id").trim();
  const clientSecret = form.watch("oauth_client_secret").trim();
  const submitDisabled =
    pending ||
    (preview.requires_gateway_url && endpointUrl.length === 0) ||
    (requiresClientCredentials &&
      (clientId.length === 0 || clientSecret.length === 0));

  return (
    <Form {...form}>
      <form className="space-y-4" onSubmit={(event) => void submit(event)}>
        {connectLinkShowsEndpointUrl(preview) ? (
          <FormField
            control={form.control}
            name="endpoint_url"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Service URL</FormLabel>
                <FormControl>
                  <Input type="url" placeholder="https://" {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        ) : null}
        {requiresClientCredentials ? (
          <>
            <FormField
              control={form.control}
              name="oauth_client_id"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>OAuth client ID</FormLabel>
                  <FormControl>
                    <Input autoComplete="off" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="oauth_client_secret"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>OAuth client secret</FormLabel>
                  <FormControl>
                    <Input type="password" autoComplete="off" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          </>
        ) : null}
        {formError ? <ErrorBanner message={formError} /> : null}
        <div>
          <Button
            className="w-full"
            type="submit"
            variant="primary"
            disabled={submitDisabled}
            isLoading={pending}
          >
            Approve &amp; continue{" "}
            <ButtonIcon variant="primary">
              <ArrowRight />
            </ButtonIcon>
          </Button>
        </div>
      </form>
    </Form>
  );
}

export function RequestDetails({
  preview,
}: {
  readonly preview: ConnectLinkPreview;
}) {
  return (
    <div className="space-y-2 border-y border-border py-4">
      <div className="space-y-1">
        <ConnectLinkDetailRow
          label="Requested by"
          value={preview.requested_by ?? "Your NyxID account"}
        />
        {preview.label ? (
          <ConnectLinkDetailRow label="Connection name" value={preview.label} />
        ) : null}
        {preview.scopes.length > 0 ? (
          <ConnectLinkDetailRow
            label="Requested permissions"
            value={preview.scopes.join(", ")}
          />
        ) : null}
      </div>
      {preview.api_key_url ? (
        <a
          className="inline-flex items-center gap-1.5 px-4 text-xs text-muted-foreground hover:text-foreground"
          href={preview.api_key_url}
          target="_blank"
          rel="noreferrer"
        >
          Credential setup{" "}
          <ExternalLink className="size-3" aria-hidden="true" />
        </a>
      ) : null}
      {preview.scopes.length > 0 &&
      (preview.connect_method === "oauth" ||
        preview.connect_method === "device_code") ? (
        <p className="px-4 text-xs text-muted-foreground">
          These additional permissions will be requested on top of the provider
          defaults.
        </p>
      ) : null}
    </div>
  );
}

export function ConnectLinkDetailRow({
  label,
  value,
  capitalizeValue = false,
}: {
  readonly label: string;
  readonly value: string;
  readonly capitalizeValue?: boolean;
}) {
  return (
    <div className="flex justify-between gap-4 px-4 py-1 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <span
        className={cn(
          "min-w-0 break-words text-right font-medium text-foreground",
          capitalizeValue && "capitalize",
        )}
      >
        {value}
      </span>
    </div>
  );
}

function DeviceCodePanel({
  code,
  url,
  interval,
  status,
}: {
  readonly code: string;
  readonly url: string;
  readonly interval: number;
  readonly status: string;
}) {
  return (
    <div className="space-y-3 rounded-lg border border-border/50 p-4">
      <p className="text-[12px] text-muted-foreground">
        Enter this code at the provider. NyxID will finish the connection after
        authorization.
      </p>
      <p className="font-mono text-[15px] font-semibold text-foreground">
        {code}
      </p>
      <p className="text-[11px] text-muted-foreground" role="status">
        {status === "slow_down"
          ? `Provider requested a slower check. Checking again in ${interval} seconds.`
          : `Checking automatically every ${interval} seconds.`}
      </p>
      <div className="flex flex-wrap justify-end gap-2">
        <Button asChild variant="outline">
          <a href={url} target="_blank" rel="noreferrer">
            Open provider
          </a>
        </Button>
      </div>
    </div>
  );
}

export function TerminalPanel({
  status,
  serviceName,
  callbackUrl,
}: {
  readonly status: "completed" | "cancelled" | "expired";
  readonly serviceName?: string;
  readonly callbackUrl: string | null;
}) {
  const completed = status === "completed";
  return (
    <section
      role="status"
      className="flex flex-col items-center py-10 text-center sm:py-16"
    >
      <div className="mb-8 flex size-20 items-center justify-center rounded-2xl border border-border bg-card">
        {completed ? (
          <NyxidIcon className="size-10" alt="" />
        ) : (
          <XCircle className="size-9 text-muted-foreground" />
        )}
      </div>
      {completed ? (
        <p className="mb-3 flex items-center gap-2 text-sm font-medium text-success">
          <CheckCircle2 className="size-4" />
          Connection completed
        </p>
      ) : null}
      <h1 className="text-[28px] font-semibold leading-tight text-foreground sm:text-[36px]">
        {completed
          ? `${serviceName ?? "Service"} connected`
          : status === "cancelled"
            ? "Connection cancelled"
            : "Connection request expired"}
      </h1>
      <p className="mt-8 text-sm text-muted-foreground sm:mt-12 sm:text-base">
        {callbackUrl
          ? "Returning to the requesting application..."
          : completed
            ? "You may now close this page."
            : "You may now close this page or return to the requesting application."}
      </p>
      {completed && !callbackUrl ? (
        <p className="mt-2 text-xs text-muted-foreground">
          Your agent can now retry the original request.
        </p>
      ) : null}
    </section>
  );
}

function ConnectShell({
  children,
  embedded = false,
}: {
  readonly children: React.ReactNode;
  readonly embedded?: boolean;
}) {
  if (embedded) {
    return <EmbeddedConnectShell>{children}</EmbeddedConnectShell>;
  }
  return <StandaloneConnectShell>{children}</StandaloneConnectShell>;
}

function EmbeddedConnectShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex w-full flex-col gap-5 text-foreground">{children}</div>
  );
}

function StandaloneConnectShell({ children }: { children: React.ReactNode }) {
  useApplyTheme();
  return (
    <main className="connect-link-shell flex min-h-dvh items-center justify-center px-4 py-8 text-foreground">
      <div className="relative z-10 flex w-full max-w-lg flex-col gap-5">
        {children}
        <p className="text-center text-xs text-muted-foreground">
          Service connection via NyxID
        </p>
      </div>
    </main>
  );
}
