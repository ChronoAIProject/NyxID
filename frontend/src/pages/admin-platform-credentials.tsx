import { useState } from "react";
import { changedFields, describeChanges, sameValue } from "@/lib/form-changes";
import { useChangeReview } from "@/components/shared/change-review-dialog";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";
import { zodResolver } from "@hookform/resolvers/zod";
import { KeyRound, RotateCw, Save, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import {
  useAdminPlatformCredentials,
  useClearPlatformCredentials,
  useUpdatePlatformCredentials,
} from "@/hooks/use-admin-platform-credentials";
import {
  platformCredentialFormSchema,
  type PlatformCredentialForm,
} from "@/schemas/admin-platform-credentials";
import type { PlatformCredentials } from "@/types/admin";
import { useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { CopyableUrlCallout } from "@/components/shared/copyable-url-callout";
import { ApiError } from "@/lib/api-client";
import { ServiceIcon } from "@/components/service-icon";

function CredentialForm({
  provider: source,
  onRefresh,
}: {
  readonly provider: PlatformCredentials;
  readonly onRefresh: () => Promise<PlatformCredentials | undefined>;
}) {
  const [refreshRequired, setRefreshRequired] = useState(false);
  const [provider, setProvider] = useState(source);
  const update = useUpdatePlatformCredentials(provider.provider);
  const clear = useClearPlatformCredentials(provider.provider);
  const [confirm, setConfirm] = useState<"clear" | "regenerate" | null>(null);
  const sharedProvider =
    provider.backing?.type === "provider_oauth"
      ? provider.backing.provider_slug
      : null;
  const form = useAppForm<PlatformCredentialForm>({
    resolver: zodResolver(platformCredentialFormSchema),
    defaultValues: credentialFormValues(provider),
    mode: "onChange",
  });
  const { isDirty, isValid } = form.formState;
  const stale = !refreshRequired && !sameValue(provider, source);
  async function save(fields: Record<string, string | null>) {
    try {
      const saved = await update.mutateAsync({ fields });
      setProvider(saved);
      form.reset(credentialFormValues(saved));
      update.reset();
      toast.success("Platform credentials saved");
    } catch (error) {
      form.setError("root", {
        message:
          error instanceof ApiError
            ? error.message
            : "Unable to save platform credentials",
      });
      throw error;
    }
  }

  const review = useChangeReview<{
    providerId: string;
    fields: Record<string, string | null>;
  }>(
    ({ providerId, fields }) =>
      providerId === provider.provider ? save(fields) : Promise.resolve(),
    stale || refreshRequired,
    provider.provider,
  );
  const pending = update.isPending || clear.isPending;
  const unavailable = pending || refreshRequired;

  function onSubmit(values: PlatformCredentialForm) {
    const before = credentialFormValues(provider).fields;
    const fields = changedFields(before, values.fields) as Record<
      string,
      string | null
    >;
    // A blank secret input keeps the stored secret; only the clear button sends null.
    for (const field of provider.fields) {
      if (fields[field.name] === "") {
        if (field.secret) delete fields[field.name];
        else fields[field.name] = null;
      }
    }
    review.review({ providerId: provider.provider, fields }, [
      ...(sharedProvider && Object.values(fields).includes(null)
        ? [
            {
              field: "Shared OAuth credentials",
              before: sharedProvider,
              after:
                provider.provider === "aurinko"
                  ? fields.client_id === null || fields.client_secret === null
                    ? "Clearing application credentials prevents new mailbox authorizations and reconnects until restored. Clearing the webhook signing secret stops managed bot webhook verification. Manual connections keep their own credentials."
                    : "Clearing the webhook signing secret stops managed bot webhook verification until restored. Application credentials and AI Service mailbox tokens are retained."
                  : `Clearing these credentials stops all of the ${sharedProvider} provider's OAuth connections and logins until credentials are restored.`,
            },
          ]
        : []),
      ...describeChanges(before, fields, {
        labels: Object.fromEntries(
          provider.fields.map((field) => [field.name, field.label]),
        ),
        secretFields: provider.fields
          .filter((field) => field.secret)
          .map((field) => field.name),
      }),
    ]);
  }

  async function confirmAction() {
    if (pending || stale || !confirm) return;
    try {
      if (confirm === "clear") {
        const { saved } = await clear.mutateAsync();
        setConfirm(null);
        form.reset({
          fields: Object.fromEntries(
            provider.fields.map((field) => [field.name, ""]),
          ),
        });
        setRefreshRequired(!saved);
        if (saved) {
          setProvider(saved);
          form.reset(credentialFormValues(saved));
        }
        toast.success("Platform credentials cleared");
      } else {
        const draft = changedFields(
          credentialFormValues(provider).fields,
          form.getValues().fields,
        );
        const saved = await update.mutateAsync({
          regenerate_verify_token: true,
        });
        setProvider(saved);
        form.reset(credentialFormValues(saved));
        for (const [name, value] of Object.entries(draft)) {
          if (value !== undefined) form.setValue(`fields.${name}`, value);
        }
      }
      update.reset();
      setConfirm(null);
    } catch (error) {
      toast.error(
        error instanceof ApiError
          ? error.message
          : "Unable to update platform credentials",
      );
    }
  }

  return (
    <section className="space-y-6 border-b border-border pb-8">
      <div className="flex items-center gap-3">
        {provider.provider === "aurinko" ? (
          <ServiceIcon slug="api-aurinko" size="xs" />
        ) : (
          <KeyRound className="size-4 text-muted-foreground" />
        )}
        <h2 className="text-[15px] font-semibold">{provider.label}</h2>
        <Badge variant={provider.available ? "success" : "secondary"}>
          {refreshRequired
            ? "Refresh required"
            : provider.available
              ? "Configured"
              : "Not configured"}
        </Badge>
      </div>
      {sharedProvider && (
        <p className="text-xs text-muted-foreground">
          Shared with the {sharedProvider} provider.
        </p>
      )}
      {refreshRequired && (
        <ErrorBanner
          message="Credentials cleared; current details could not be refreshed"
          onRetry={async () => {
            const saved = await onRefresh();
            if (saved) {
              setProvider(saved);
              form.reset(credentialFormValues(saved));
              setRefreshRequired(false);
            }
          }}
        />
      )}
      {stale && (
        <StaleFormNotice
          onReload={() => {
            setProvider(source);
            form.reset(credentialFormValues(source));
            review.cancel();
          }}
        />
      )}
      {review.dialog}
      <form
        onSubmit={form.handleSubmit(onSubmit)}
        className="max-w-2xl space-y-4"
      >
        {form.formState.errors.root && (
          <ErrorBanner
            message={form.formState.errors.root.message ?? "Unable to save"}
          />
        )}
        {provider.fields.map((field) => (
          <div key={field.name} className="space-y-2">
            <div className="flex items-center justify-between gap-2">
              <Label htmlFor={`${provider.provider}-${field.name}`}>
                {field.label}
              </Label>
              {!refreshRequired && field.configured && (
                <Badge variant="success">Configured</Badge>
              )}
            </div>
            <div className="flex items-start gap-2">
              <Input
                id={`${provider.provider}-${field.name}`}
                type={field.secret ? "password" : "text"}
                inputMode={field.numeric ? "numeric" : undefined}
                autoComplete="off"
                disabled={unavailable}
                placeholder={
                  field.secret && field.configured
                    ? "Enter a replacement to rotate"
                    : undefined
                }
                {...form.register(`fields.${field.name}`)}
              />
              <Button
                type="button"
                size="icon"
                variant="ghost"
                title={`Clear ${field.label}`}
                aria-label={`Clear ${field.label}`}
                disabled={unavailable || !field.configured}
                onClick={() => form.setValue(`fields.${field.name}`, null)}
              >
                <X className="size-3" />
              </Button>
            </div>
            {form.watch(`fields.${field.name}`) === null && (
              <p className="text-xs text-destructive">
                Will be cleared when you confirm changes.
              </p>
            )}
            <p className="text-xs text-muted-foreground">{field.help}</p>
            {form.formState.errors.fields?.[field.name] && (
              <p className="text-xs text-destructive">
                {form.formState.errors.fields[field.name]?.message}
              </p>
            )}
          </div>
        ))}
        <div className="flex flex-wrap gap-2">
          <Button
            variant="primary"
            type="submit"
            isLoading={pending}
            disabled={unavailable || stale || !isDirty || !isValid}
          >
            <Save className="size-3" />
            Save credentials
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={unavailable || stale || !provider.updated_at}
            onClick={() => setConfirm("clear")}
          >
            <Trash2 className="size-3" />
            Clear provider
          </Button>
        </div>
      </form>
      <div className="max-w-2xl space-y-3">
        {provider.callback_url && (
          <CopyableUrlCallout
            label="Platform Callback URL"
            url={provider.callback_url}
          />
        )}
        {provider.webhook_verify_token && (
          <>
            <CopyableUrlCallout
              label="Platform Verify Token"
              url={provider.webhook_verify_token}
            />
            <Button
              variant="outline"
              disabled={unavailable || stale}
              onClick={() => setConfirm("regenerate")}
            >
              <RotateCw className="size-3" />
              Regenerate verify token
            </Button>
          </>
        )}
      </div>
      <div className="max-w-2xl space-y-3">
        <h3 className="text-[13px] font-medium">Setup checklist</h3>
        <ol className="list-decimal space-y-3 pl-4 text-xs text-muted-foreground">
          {provider.setup_checklist.map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      </div>
      <Dialog open={confirm !== null} onOpenChange={() => setConfirm(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {confirm !== "regenerate"
                ? "Clear platform credentials"
                : "Regenerate verify token"}
            </DialogTitle>
            <DialogDescription>
              {confirm === "clear"
                ? provider.provider === "aurinko"
                  ? "Clearing these credentials prevents managed mailbox authorization and managed bot webhook verification until restored. Manual connections keep their own credentials. Unsaved credential edits will be discarded."
                  : sharedProvider
                    ? `These credentials are shared with the ${sharedProvider} provider. Clearing them stops all of its OAuth connections and logins until credentials are restored. Unsaved credential edits will be discarded.`
                    : "Managed onboarding and managed bot authentication will be unavailable until credentials are restored. Unsaved credential edits will be discarded."
                : `Update ${provider.label}'s webhook verification settings with the replacement token.`}
            </DialogDescription>
          </DialogHeader>
          {stale && (
            <p role="alert">
              Saved values changed. Cancel and load the latest values before
              continuing.
            </p>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirm(null)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              isLoading={pending}
              disabled={unavailable || stale}
              onClick={() => void confirmAction()}
            >
              Confirm
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

export function AdminPlatformCredentialsPage() {
  const query = useAdminPlatformCredentials();
  return (
    <div className="space-y-8">
      <PageHeader title="Platform Credentials" />
      {query.error && (
        <ErrorBanner
          message="Unable to refresh platform credentials"
          onRetry={query.refetch}
        />
      )}
      {query.isLoading && !query.data ? (
        <Skeleton className="h-64 w-full" />
      ) : (
        query.data?.map((provider) => (
          <CredentialForm
            key={provider.provider}
            provider={provider}
            onRefresh={async () => {
              const result = await query.refetch();
              return result.error
                ? undefined
                : result.data?.find(
                    (item) => item.provider === provider.provider,
                  );
            }}
          />
        ))
      )}
    </div>
  );
}

function credentialFormValues(
  provider: PlatformCredentials,
): PlatformCredentialForm {
  return {
    fields: Object.fromEntries(
      provider.fields.map((field) => [
        field.name,
        field.secret ? "" : (field.value ?? ""),
      ]),
    ),
  };
}
