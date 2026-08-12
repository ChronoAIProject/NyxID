import { useEffect, useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useNavigate, useParams } from "@tanstack/react-router";
import {
  CheckCircle2,
  ExternalLink,
  KeyRound,
  ShieldCheck,
  XCircle,
} from "lucide-react";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
        : cancel.data?.callback_url ?? preview.data?.callback_url;
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
    if (connectLinkNeedsSetupForm(preview.data)) {
      setShowSetupForm(true);
      return;
    }
    await submitCompletion();
  }

  async function submitCompletion(values?: CompleteConnectLinkInput) {
    setSubmitError(null);
    try {
      const result = await complete.mutateAsync({ token, values });
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
    if (!deviceChallenge?.state || actionPending || withinCooldown())
      return;
    void submitCompletion({ device_state: deviceChallenge.state });
  }

  if (isLoading || !isAuthenticated) {
    return (
      <ConnectShell>
        <Skeleton className="h-72 w-full" />
      </ConnectShell>
    );
  }

  const terminal = complete.data?.status === "completed"
    ? { status: "completed" as const, callbackUrl: complete.data.callback_url }
    : cancel.data && cancel.data.status !== "pending"
      ? { status: cancel.data.status, callbackUrl: cancel.data.callback_url }
      : preview.data && preview.data.status !== "pending"
        ? { status: preview.data.status, callbackUrl: preview.data.callback_url }
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
      endpoint_url: "",
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

  return (
    <Form {...form}>
      <form className="space-y-4" onSubmit={(event) => void submit(event)}>
        <FormField
          control={form.control}
          name="credential"
          render={({ field }) => (
            <FormItem>
              <FormLabel>{preview.auth_key_name}</FormLabel>
              <FormControl>
                <Input type="password" autoComplete="off" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        {preview.requires_gateway_url ? (
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
        {formError ? <ErrorBanner message={formError} /> : null}
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            disabled={submitDisabled}
            isLoading={pending}
          >
            <ButtonIcon variant="primary">
              <KeyRound />
            </ButtonIcon>
            Connect
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
      endpoint_url: "",
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
        {preview.requires_gateway_url ? (
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
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            disabled={submitDisabled}
            isLoading={pending}
          >
            <ButtonIcon variant="primary">
              <KeyRound />
            </ButtonIcon>
            Continue
          </Button>
        </div>
      </form>
    </Form>
  );
}

function RequestDetails({ preview }: { readonly preview: ConnectLinkPreview }) {
  return (
    <div className="divide-y divide-border/30 rounded-lg border border-border/50 bg-white/[0.02]">
      <ConnectLinkDetailRow label="Service" value={preview.service_name} />
      <ConnectLinkDetailRow
        label="Requested by"
        value={preview.requested_by ?? "Your NyxID account"}
      />
      <ConnectLinkDetailRow
        label="Label"
        value={preview.label ?? "Not provided"}
      />
      <ConnectLinkDetailRow
        label="Created"
        value={new Date(preview.created_at).toLocaleString()}
      />
      <ConnectLinkDetailRow
        label="Status"
        value={preview.status}
        capitalizeValue
      />
      {preview.api_key_url ? (
        <div className="flex items-center justify-between gap-4 px-4 py-2.5 text-[12px]">
          <span className="text-muted-foreground">Credential source</span>
          <a
            className="inline-flex items-center gap-1 text-nyx-secondary-400 hover:underline"
            href={preview.api_key_url}
            target="_blank"
            rel="noreferrer"
          >
            Open provider <ExternalLink className="h-3 w-3" />
          </a>
        </div>
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
    <div className="flex justify-between gap-4 px-4 py-2.5 text-[12px]">
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
  pending,
  onCheck,
}: {
  readonly code: string;
  readonly url: string;
  readonly pending: boolean;
  readonly onCheck: () => void;
}) {
  return (
    <div className="space-y-3 rounded-lg border border-border/50 p-4">
      <p className="text-[12px] text-muted-foreground">
        Enter this code at the provider, then check the connection.
      </p>
      <p className="font-mono text-[15px] font-semibold text-foreground">
        {code}
      </p>
      <div className="flex flex-wrap justify-end gap-2">
        <Button asChild variant="outline">
          <a href={url} target="_blank" rel="noreferrer">
            Open provider
          </a>
        </Button>
        <Button
          type="button"
          variant="primary"
          disabled={pending}
          isLoading={pending}
          onClick={onCheck}
        >
          Check connection
        </Button>
      </div>
    </div>
  );
}

export function TerminalPanel({
  status,
  callbackUrl,
}: {
  readonly status: "completed" | "cancelled" | "expired";
  readonly callbackUrl: string | null;
}) {
  const completed = status === "completed";
  return (
    <Card
      className={completed ? "border-success/25 bg-success/[0.03]" : "border-border/50"}
    >
      <CardContent className="flex flex-col items-center gap-3 p-6 text-center">
        {completed ? (
          <CheckCircle2 className="h-6 w-6 text-success" />
        ) : (
          <XCircle className="h-6 w-6 text-muted-foreground" />
        )}
        <h2 className="text-[15px] font-semibold text-foreground">
          {completed
            ? "Service connected"
            : status === "cancelled"
              ? "Connection cancelled"
              : "Connection request expired"}
        </h2>
        <p className="text-[12px] text-muted-foreground">
          {completed
            ? "Return to your agent. It can now retry the original request."
            : "No credential was connected. Return to the requesting application."}
        </p>
        {callbackUrl ? (
          <p className="text-[11px] text-muted-foreground">
            Returning to the requesting application...
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}

function ConnectShell({ children }: { readonly children: React.ReactNode }) {
  return (
    <main className="flex min-h-dvh items-start justify-center bg-background px-4 py-8 text-foreground sm:items-center">
      <div className="flex w-full max-w-xl flex-col gap-5">{children}</div>
    </main>
  );
}
