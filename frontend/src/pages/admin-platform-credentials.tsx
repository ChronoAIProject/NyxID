import { useEffect, useRef, useState } from "react";
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

function CredentialForm({
  provider,
}: {
  readonly provider: PlatformCredentials;
}) {
  const update = useUpdatePlatformCredentials(provider.provider);
  const clear = useClearPlatformCredentials(provider.provider);
  const [confirm, setConfirm] = useState<"clear" | "regenerate" | { field: string } | null>(null);
  const sharedProvider = provider.backing?.type === "provider_oauth"
    ? provider.backing.provider_slug
    : null;
  const form = useAppForm<PlatformCredentialForm>({
    resolver: zodResolver(platformCredentialFormSchema),
    defaultValues: { fields: {} },
    mode: "onChange",
  });
  const { reset } = form;
  const lastRevision = useRef<string | null>(null);
  useEffect(() => {
    const revision = JSON.stringify([provider.provider, provider.updated_at, provider.fields]);
    if (lastRevision.current === revision) return;
    lastRevision.current = revision;
    reset({
      fields: Object.fromEntries(
        provider.fields.map((field) => [
          field.name,
          field.secret ? "" : (field.value ?? ""),
        ]),
      ),
    });
  }, [provider, reset]);
  const pending = update.isPending || clear.isPending;

  async function save(values: PlatformCredentialForm) {
    const fields = Object.fromEntries(
      Object.entries(values.fields).filter(
        ([name]) => form.formState.dirtyFields.fields?.[name],
      ),
    );
    try {
      await update.mutateAsync({ fields });
      update.reset();
      toast.success("Platform credentials saved");
    } catch (error) {
      form.setError("root", {
        message:
          error instanceof ApiError
            ? error.message
            : "Unable to save platform credentials",
      });
    }
  }

  async function confirmAction() {
    try {
      if (confirm && typeof confirm === "object") {
        await update.mutateAsync({ fields: { [confirm.field]: null } });
      } else if (confirm === "clear") await clear.mutateAsync();
      else await update.mutateAsync({ regenerate_verify_token: true });
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
        <KeyRound className="size-4 text-muted-foreground" />
        <h2 className="text-[15px] font-semibold">{provider.label}</h2>
        <Badge variant={provider.available ? "success" : "secondary"}>
          {provider.available ? "Configured" : "Not configured"}
        </Badge>
      </div>
      {provider.backing?.type === "provider_oauth" && (
        <p className="text-xs text-muted-foreground">Shared with the {provider.backing.provider_slug} provider.</p>
      )}
      <form onSubmit={form.handleSubmit(save)} className="max-w-2xl space-y-4">
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
              {field.configured && <Badge variant="success">Configured</Badge>}
            </div>
            <div className="flex items-start gap-2">
              <Input
                id={`${provider.provider}-${field.name}`}
                type={field.secret ? "password" : "text"}
                inputMode={field.numeric ? "numeric" : undefined}
                autoComplete="off"
                disabled={pending}
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
                disabled={pending || !field.configured}
                onClick={() => sharedProvider
                  ? setConfirm({ field: field.name })
                  : form.setValue(`fields.${field.name}`, null)}
              >
                <X className="size-3" />
              </Button>
            </div>
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
            disabled={!form.formState.isDirty || !form.formState.isValid}
          >
            <Save className="size-3" />
            Save credentials
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={pending || !provider.updated_at}
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
              disabled={pending}
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
              {confirm !== "regenerate"
                ? sharedProvider
                  ? `These credentials are shared with the ${sharedProvider} provider. Clearing them stops all of its OAuth connections and logins until credentials are restored.`
                  : "Managed onboarding and managed bot authentication will be unavailable until credentials are restored."
                : `Update ${provider.label}'s webhook verification settings with the replacement token.`}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirm(null)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              isLoading={pending}
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
      {query.isLoading ? (
        <Skeleton className="h-64 w-full" />
      ) : query.error ? (
        <ErrorBanner
          message="Unable to load platform credentials"
          onRetry={query.refetch}
        />
      ) : (
        query.data?.map((provider) => (
          <CredentialForm key={provider.provider} provider={provider} />
        ))
      )}
    </div>
  );
}
