import { useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { KeyRound, ShieldCheck, XCircle } from "lucide-react";
import {
  CredentialForm,
  OAuthSetupForm,
  RequestDetails,
  DeviceCodePanel,
  TerminalPanel,
  ConnectShell,
} from "@/components/connect/connection-panels";
import { CredentialBindingChoice } from "@/components/shared/credential-binding-choice";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  connectLinkStorageKey,
  useCancelHostedConnectLink,
  useCompleteConnectLink,
  useConnectLinkStatus,
  usePreviewConnectLink,
} from "@/hooks/use-connect-links";
import { useCatalogEntry } from "@/hooks/use-keys";
import {
  connectLinkErrorMessage,
  connectLinkNeedsSetupForm,
  connectLinkProviderError,
} from "@/lib/connect-link-page";
import {
  type CompleteConnectLinkInput,
  type ConnectCredentialForm,
  type ConnectOAuthForm,
} from "@/schemas/connect-links";
import { useAuthStore } from "@/stores/auth-store";

const CLICK_THROTTLE_MS = 750;

interface DeviceChallenge {
  readonly code: string;
  readonly url: string;
  readonly state: string;
}

export function ConnectLinkPage() {
  const { token } = useParams({ strict: false }) as { token: string };
  const navigate = useNavigate();
  const { isAuthenticated, isLoading } = useAuthStore();
  const [showSetupForm, setShowSetupForm] = useState(false);
  const [deviceChallenge, setDeviceChallenge] =
    useState<DeviceChallenge | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const lastClickAtRef = useRef(0);
  const preview = usePreviewConnectLink();
  const [platformChoice, setPlatformChoice] = useState<boolean | null>(null);
  const { data: catalog } = useCatalogEntry(
    isAuthenticated ? preview.data?.service_slug : undefined,
  );
  const platformAvailable = Boolean(
    catalog?.platform_key?.available && !preview.data?.scopes.length,
  );
  const usePlatformKey = platformAvailable &&
    (platformChoice ?? preview.data?.use_platform_key ?? true);
  const complete = useCompleteConnectLink();
  const cancel = useCancelHostedConnectLink();
  const actionPending =
    preview.isPending || complete.isPending || cancel.isPending;

  useEffect(() => {
    if (isLoading || isAuthenticated) return;
    const returnTo = `${window.location.origin}/connect/${encodeURIComponent(token)}`;
    void navigate({ to: "/login", search: { return_to: returnTo } });
  }, [isAuthenticated, isLoading, navigate, token]);

  useEffect(() => {
    const callback =
      complete.data?.status === "completed"
        ? complete.data.callback_url
        : (cancel.data?.callback_url ?? preview.data?.callback_url);
    if (!callback) return;
    const timer = window.setTimeout(
      () => window.location.assign(callback),
      1_500,
    );
    return () => window.clearTimeout(timer);
  }, [cancel.data, complete.data, preview.data]);

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

  async function handleConnect() {
    if (!preview.data || actionPending || withinCooldown()) return;
    if (!usePlatformKey && connectLinkNeedsSetupForm(preview.data)) {
      setShowSetupForm(true);
      return;
    }
    await submitCompletion();
  }

  async function submitCompletion(values?: CompleteConnectLinkInput) {
    setSubmitError(null);
    try {
      const selected = { use_platform_key: usePlatformKey };
      const result = await complete.mutateAsync({
        token,
        values: usePlatformKey ? selected : { ...values, ...selected },
      });
      if (result.status === "oauth_required" && result.authorization_url) {
        sessionStorage.setItem(connectLinkStorageKey(result.id), token);
        window.location.assign(result.authorization_url);
        return;
      }
      if (result.status === "device_code_required") {
        setShowSetupForm(false);
        setDeviceChallenge((current) => ({
          code: result.device_user_code ?? current?.code ?? "",
          url: result.device_verification_uri ?? current?.url ?? "",
          state: result.device_state ?? current?.state ?? "",
        }));
        return;
      }
      setDeviceChallenge(null);
    } catch (error) {
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

  function handleDeviceCheck() {
    if (!deviceChallenge?.state || actionPending || withinCooldown()) return;
    void submitCompletion({ device_state: deviceChallenge.state });
  }

  if (isLoading || !isAuthenticated) {
    return (
      <ConnectShell>
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
    <ConnectShell>
      <header className="space-y-2 text-center">
        <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-lg border border-nyx-500/30 bg-nyx-500/10">
          <KeyRound className="h-4 w-4 text-nyx-secondary-400" />
        </div>
        <h1 className="text-[22px] font-bold leading-tight text-foreground sm:text-[28px]">
          Connect a service
        </h1>
      </header>

      {terminal ? (
        <TerminalPanel
          status={terminal.status}
          callbackUrl={terminal.callbackUrl ?? null}
        />
      ) : (
        <Card className="border-border/50">
          <CardHeader>
            <CardTitle>Connection request</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {submitError ? <ErrorBanner message={submitError} /> : null}
            {!preview.data ? (
              <div className="space-y-4">
                <p className="text-[12px] leading-relaxed text-muted-foreground">
                  Review who requested this connection before sharing a
                  credential.
                </p>
                <div className="flex justify-end">
                  <Button
                    type="button"
                    variant="primary"
                    disabled={actionPending}
                    isLoading={preview.isPending}
                    onClick={() => void handlePreview()}
                  >
                    <ButtonIcon variant="primary">
                      <ShieldCheck />
                    </ButtonIcon>
                    Review request
                  </Button>
                </div>
              </div>
            ) : (
              <>
                <RequestDetails preview={preview.data} />
                {platformAvailable && catalog?.platform_key && preview.data.status === "pending" && (
                  <CredentialBindingChoice
                    value={usePlatformKey}
                    onChange={(value) => {
                      setPlatformChoice(value);
                      setShowSetupForm(false);
                    }}
                    platformPrice={catalog.platform_key.pricing}
                    byokPrice={catalog.byok_pricing}
                    legacyBillable={catalog.billing?.platform_billable}
                    resaleBillable={catalog.billing?.resale_billable}
                    disabled={actionPending}
                  />
                )}
                {preview.data.status !== "pending" ? (
                  <ErrorBanner
                    message={`This connection request is ${preview.data.status}.`}
                  />
                ) : null}
                {showSetupForm ? (
                  preview.data.connect_method === "api_key" ? (
                    <CredentialForm
                      preview={preview.data}
                      pending={actionPending}
                      onSubmit={handleCredentialSubmit}
                    />
                  ) : (
                    <OAuthSetupForm
                      preview={preview.data}
                      pending={actionPending}
                      onSubmit={handleOAuthSubmit}
                    />
                  )
                ) : deviceChallenge ? null : (
                  <div className="flex justify-end">
                    <Button
                      type="button"
                      variant="primary"
                      disabled={
                        actionPending || preview.data.status !== "pending"
                      }
                      isLoading={complete.isPending}
                      onClick={() => void handleConnect()}
                    >
                      <ButtonIcon variant="primary">
                        <KeyRound />
                      </ButtonIcon>
                      Connect
                    </Button>
                  </div>
                )}
                {deviceChallenge ? (
                  <DeviceCodePanel
                    code={deviceChallenge.code}
                    url={deviceChallenge.url}
                    pending={actionPending}
                    onCheck={handleDeviceCheck}
                  />
                ) : null}
                {preview.data.status === "pending" ? (
                  <div className="flex justify-start">
                    <Button
                      type="button"
                      variant="destructive"
                      disabled={actionPending}
                      isLoading={cancel.isPending}
                      onClick={() => void handleCancel()}
                    >
                      <ButtonIcon variant="destructive">
                        <XCircle />
                      </ButtonIcon>
                      Cancel request
                    </Button>
                  </div>
                ) : null}
              </>
            )}
          </CardContent>
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
  const providerError = connectLinkProviderError(window.location.search);

  useEffect(() => {
    if (isLoading || isAuthenticated) return;
    const returnTo = `${window.location.origin}/connect/return/${linkId}`;
    void navigate({ to: "/login", search: { return_to: returnTo } });
  }, [isAuthenticated, isLoading, linkId, navigate]);

  useEffect(() => {
    if (
      providerError ||
      status.data?.status !== "pending" ||
      recoveryAttemptedRef.current
    ) {
      return;
    }
    const token = sessionStorage.getItem(connectLinkStorageKey(linkId));
    if (!token) return;
    recoveryAttemptedRef.current = true;
    void complete
      .mutateAsync({ token })
      .then((result) => {
        if (result.status === "completed") void status.refetch();
      })
      .catch((error) => setRecoveryError(connectLinkErrorMessage(error)));
  }, [complete, linkId, providerError, status]);

  useEffect(() => {
    if (
      status.data?.status !== "completed" &&
      status.data?.status !== "cancelled" &&
      status.data?.status !== "expired"
    )
      return;
    sessionStorage.removeItem(connectLinkStorageKey(linkId));
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
