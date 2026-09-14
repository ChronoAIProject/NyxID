import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { CheckCircle2, ExternalLink, KeyRound, XCircle } from "lucide-react";
import { ErrorBanner } from "@/components/shared/error-banner";
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
import { connectLinkNeedsOAuthCredentials } from "@/lib/connect-link-page";
import { cn } from "@/lib/utils";
import {
  connectCredentialFormSchema,
  connectOAuthFormSchema,
  validateConnectCredentialForm,
  validateConnectOAuthForm,
  type ConnectCredentialForm,
  type ConnectOAuthForm,
  type ConnectLinkPreview,
  type ConnectFormMetadata,
} from "@/schemas/connect-links";

export function CredentialForm({
  preview,
  pending,
  onSubmit,
}: {
  readonly preview: ConnectFormMetadata;
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

export function OAuthSetupForm({
  preview,
  pending,
  onSubmit,
}: {
  readonly preview: ConnectFormMetadata;
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

export function RequestDetails({
  preview,
}: {
  readonly preview: ConnectLinkPreview;
}) {
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

export function DeviceCodePanel({
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
      className={
        completed ? "border-success/25 bg-success/[0.03]" : "border-border/50"
      }
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

export function ConnectShell({
  children,
}: {
  readonly children: React.ReactNode;
}) {
  return (
    <main className="flex min-h-dvh items-start justify-center bg-background px-4 py-8 text-foreground sm:items-center">
      <div className="flex w-full max-w-xl flex-col gap-5">{children}</div>
    </main>
  );
}
