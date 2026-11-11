import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import {
  useSandboxBillingSettings,
  useUpdateSandboxBillingSettings,
} from "@/hooks/use-sandbox-billing-settings";
import { useServiceAccounts } from "@/hooks/use-service-accounts";
import {
  sandboxProducerFormSchema,
  type SandboxProducerForm,
  type SandboxBillingSettings as Settings,
} from "@/schemas/sandbox-billing-settings";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  FormSubmitErrors,
  useAppForm,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";

export function SandboxBillingSettings({
  serviceId,
}: {
  readonly serviceId: string;
}) {
  useUiTranslation();
  const query = useSandboxBillingSettings(serviceId);
  return (
    <section className="mt-6 space-y-3 rounded-lg border border-border p-4">
      <h2 className="type-row-title">{translateUi("Sandbox billing")}</h2>
      <p className="text-13 text-muted-foreground">{translateUi("Choose the trusted account that requests prepaid Sandbox windows and submits usage evidence. Admission follows the CMA Sandbox billing feature flag.")}</p>
      {query.isPending ? (
        <p className="text-13">{translateUi("Loading Sandbox settings…")}</p>
      ) : query.isError ? (
        <p role="alert" className="text-13 text-destructive">{translateUi("Could not load Sandbox settings.")}{" "}
          <Button
            type="button"
            variant="outline"
            onClick={() => void query.refetch()}
          >{translateUi("Retry")}</Button>
        </p>
      ) : (
        <ProducerEditor settings={query.data} />
      )}
      <p className="text-13 text-muted-foreground">{translateUi("Prices and their version history remain in")}{" "}
        <a className="underline" href="/admin/billing?tab=settings">{translateUi("Billing Settings → CMA rate cards")}</a>
        .
      </p>
    </section>
  );
}

function ProducerEditor({ settings }: { readonly settings: Settings }) {
  useUiTranslation();
  const [revision, setRevision] = useState(settings.revision);
  const [search, setSearch] = useState("");
  const accounts = useServiceAccounts(1, 100, search || undefined);
  const mutation = useUpdateSandboxBillingSettings(settings.service_id);
  const form = useAppForm<SandboxProducerForm>({
    resolver: zodResolver(sandboxProducerFormSchema),
    defaultValues: {
      producer: settings.authorized_producer_service_account_id ?? "",
    },
  });
  const selected = form.watch("producer");
  const eligible = (accounts.data?.service_accounts ?? []).filter(
    (account) =>
      account.is_active &&
      (account.purpose ?? "general") === "general" &&
      account.allowed_scopes.split(/\s+/).includes("billing:sandbox:write"),
  );
  const stale = revision !== settings.revision;
  async function save(value: SandboxProducerForm) {
    try {
      const saved = await mutation.mutateAsync({
        authorized_producer_service_account_id: value.producer || null,
        expected_revision: revision,
      });
      setRevision(saved.revision);
      form.reset({
        producer: saved.authorized_producer_service_account_id ?? "",
      });
      toast.success(translateUi("Sandbox billing producer saved"));
    } catch (error) {
      form.setError("root", {
        message:
          error instanceof Error
            ? error.message
            : "Could not save Sandbox settings",
      });
    }
  }
  return (
    <Form {...form}>
      <form className="space-y-3" onSubmit={form.handleSubmit(save)}>
        {stale && (
          <StaleFormNotice
            onReload={() => {
              setRevision(settings.revision);
              form.reset({
                producer: settings.authorized_producer_service_account_id ?? "",
              });
            }}
          />
        )}
        <Label htmlFor="sandbox-producer-search">{translateUi("Search service accounts")}</Label>
        <Input
          id="sandbox-producer-search"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder={translateUi("Search by name")}
        />
        <FormField
          control={form.control}
          name="producer"
          render={({ field }) => (
            <FormItem>
              <FormLabel>{translateUi("Authorized producer service account")}</FormLabel>
              <FormControl>
                <NativeSelect
                  {...field}
                  disabled={accounts.isPending || accounts.isError}
                >
                  <option value="">{translateUi("Not configured")}</option>
                  {selected &&
                    !eligible.some((account) => account.id === selected) && (
                      <option value={selected}>
                        {selected}{translateUi(" (current selection)")}</option>
                    )}
                  {eligible.map((account) => (
                    <option key={account.id} value={account.id}>
                      {account.name} ({account.id})
                    </option>
                  ))}
                </NativeSelect>
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        {accounts.isError && (
          <p role="alert" className="text-13 text-destructive">{translateUi("Could not load service accounts.")}{" "}
            <Button
              type="button"
              variant="outline"
              onClick={() => void accounts.refetch()}
            >{translateUi("Retry")}</Button>
          </p>
        )}
        <p className="text-13 text-muted-foreground">{translateUi("Only active General accounts with billing:sandbox:write are eligible. Saving restricts account and credential management to platform admins. When unset, Sandbox billing requests are refused. Submit evidence for funded windows before replacing the producer; rotate its credentials to keep the same identity.")}</p>
        {form.formState.errors.root && (
          <p role="alert" className="text-13 text-destructive">
            {form.formState.errors.root.message}
          </p>
        )}
        <FormSubmitErrors />
        <Button
          type="submit"
          disabled={
            !form.formState.isDirty ||
            mutation.isPending ||
            stale ||
            accounts.isError
          }
        >
          {mutation.isPending ? translateUi("Saving…") : translateUi("Save Sandbox producer")}
        </Button>
      </form>
    </Form>
  );
}
