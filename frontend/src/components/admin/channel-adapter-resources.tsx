import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAuthStore } from "@/stores/auth-store";
import { useServices } from "@/hooks/use-services";
import { useAdminUsers } from "@/hooks/use-admin";
import { useAdminAdapterResources } from "@/hooks/use-channel-adapters";
import {
  adapterAccountInputSchema,
  adapterAllocationInputSchema,
  type AdapterAccountInput,
  type AdapterAllocationInput,
} from "@/schemas/channel-adapters";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Checkbox } from "@/components/ui/checkbox";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";

export function ChannelAdapterResources() {
  const user = useAuthStore((s) => s.user);
  return user?.is_admin ? <ResourceManagement key={user.id} /> : null;
}
function ResourceManagement() {
  const data = useAdminAdapterResources();
  const services = useServices();
  const [search, setSearch] = useState("");
  const users = useAdminUsers(1, 50, search, "person");
  const [error, setError] = useState<string>();
  const [remove, setRemove] = useState<{
    kind: "account" | "allocation";
    id: string;
    name: string;
  }>();
  const account = useAppForm<AdapterAccountInput>({
    resolver: zodResolver(adapterAccountInputSchema),
    mode: "onChange",
    defaultValues: {
      adapter_id: "telnyx.messaging",
      catalog_service_id: "",
      account_id: "",
      webhook_public_key: null,
    },
  });
  const allocation = useAppForm<AdapterAllocationInput>({
    resolver: zodResolver(adapterAllocationInputSchema),
    mode: "onChange",
    defaultValues: {
      setup: {
        adapter_id: "telnyx.messaging",
        credential: { kind: "platform", catalog_service_id: "" },
        account_id: "",
        resource_id: "",
        phone_number: "",
        application_id: "",
      },
      owner_id: "",
      tariff_version: "",
      tariff_source: "",
      includes_all_provider_charges: false,
    },
  });
  const accountProfile = account.watch("adapter_id");
  const allocationProfile = allocation.watch("setup.adapter_id");
  const availableServices =
    services.data?.filter(
      (s) => s.is_active && s.slug.includes(accountProfile.split(".")[0] ?? ""),
    ) ?? [];
  async function act(task: () => Promise<unknown>) {
    setError(undefined);
    try {
      await task();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Provider resource action failed.",
      );
    }
  }
  return (
    <section
      aria-label="Platform phone accounts and allocations"
      className="space-y-6 rounded-xl border border-border bg-card p-5 text-12"
    >
      <div>
        <h2 className="text-16 font-semibold">
          Platform phone accounts and allocations
        </h2>
        <p className="mt-2 text-muted-foreground">
          Trust a provider account, then allocate its verified number and
          application to one person. Catalog platform-key access does not grant
          ownership of a phone resource. Telnyx platform voice is unavailable
          because its inbound API cannot enforce a call-leg time cap.
        </p>
      </div>
      {(data.accounts.error || data.allocations.error) && (
        <ErrorBanner
          message="Could not load platform phone resources."
          onRetry={() => {
            void data.accounts.refetch();
            void data.allocations.refetch();
          }}
        />
      )}
      <div className="space-y-3">
        <h3 className="font-medium">Trusted accounts</h3>
        {data.accounts.data?.map((a) => (
          <article
            key={a.id}
            className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border p-3"
          >
            <div>
              {a.provider} · {a.account_id}
              <p className="text-muted-foreground">
                Revision {a.revision} ·{" "}
                {a.webhook_key_configured
                  ? "Verification key configured"
                  : "Provider credential authentication"}
              </p>
            </div>
            <Button
              variant="outline"
              onClick={() =>
                setRemove({ kind: "account", id: a.id, name: a.account_id })
              }
            >
              Remove trust
            </Button>
          </article>
        ))}
        {data.accounts.hasNextPage && (
          <Button
            variant="outline"
            disabled={data.accounts.isFetchingNextPage}
            onClick={() => void data.accounts.fetchNextPage()}
          >
            Load more trusted accounts
          </Button>
        )}
        <details>
          <summary className="cursor-pointer">
            Configure or rotate account trust
          </summary>
          <Form {...account}>
            <form
              className="mt-4 space-y-3"
              onSubmit={account.handleSubmit(
                (v) =>
                  void act(async () => {
                    await data.configure.mutateAsync(v);
                    account.reset();
                  }),
              )}
            >
              <FormField
                control={account.control}
                name="adapter_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Provider</FormLabel>
                    <Select
                      value={field.value}
                      onValueChange={(v) => {
                        field.onChange(v);
                        account.setValue("catalog_service_id", "");
                        account.setValue("webhook_public_key", null);
                      }}
                    >
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="telnyx.messaging">Telnyx</SelectItem>
                        <SelectItem value="twilio.messaging">Twilio</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={account.control}
                name="catalog_service_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Platform catalog service</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder="Choose catalog service" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        {availableServices.map((s) => (
                          <SelectItem key={s.id} value={s.id}>
                            {s.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={account.control}
                name="account_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Provider account ID</FormLabel>
                    <FormControl>
                      <Input {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              {accountProfile.startsWith("telnyx.") && (
                <FormField
                  control={account.control}
                  name="webhook_public_key"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Account Ed25519 public key</FormLabel>
                      <FormControl>
                        <Input {...field} value={field.value ?? ""} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}
              <p className="text-muted-foreground">
                Configure the encrypted platform credential and ACL in the
                catalog first. Rotating trust invalidates existing bindings
                until owners verify them again.
              </p>
              <Button
                type="submit"
                disabled={
                  !account.formState.isDirty ||
                  !account.formState.isValid ||
                  data.configure.isPending
                }
              >
                Save trusted account
              </Button>
            </form>
          </Form>
        </details>
      </div>
      <div className="space-y-3">
        <h3 className="font-medium">Resource allocations</h3>
        {data.allocations.data?.map((a) => (
          <article
            key={a.id}
            className="space-y-2 rounded-lg border border-border p-3"
          >
            <div className="flex flex-wrap justify-between gap-3">
              <p>
                {a.provider} · {a.phone_number}
              </p>
              <Button
                variant="outline"
                onClick={() =>
                  setRemove({
                    kind: "allocation",
                    id: a.id,
                    name: a.phone_number,
                  })
                }
              >
                Revoke allocation
              </Button>
            </div>
            <p>
              Owner:{" "}
              {users.data?.users.find((u) => u.id === a.owner_id)?.email ??
                a.owner_id}
            </p>
            <p className="text-muted-foreground">
              Number {a.resource_id} · Application {a.application_id}
            </p>
            <p>
              Tariff {a.tariff_version} ·{" "}
              <a
                href={a.tariff_source}
                target="_blank"
                rel="noreferrer"
                className="underline"
              >
                Evidence
              </a>{" "}
              ·{" "}
              {a.includes_all_provider_charges
                ? "All provider charges covered"
                : "Billing setup incomplete"}
            </p>
          </article>
        ))}
        {data.allocations.hasNextPage && (
          <Button
            variant="outline"
            disabled={data.allocations.isFetchingNextPage}
            onClick={() => void data.allocations.fetchNextPage()}
          >
            Load more allocations
          </Button>
        )}
        <details>
          <summary className="cursor-pointer">
            Allocate a verified number and application
          </summary>
          <Form {...allocation}>
            <form
              className="mt-4 space-y-3"
              onSubmit={allocation.handleSubmit(
                (v) =>
                  void act(async () => {
                    await data.allocate.mutateAsync(v);
                    allocation.reset();
                  }),
              )}
            >
              <FormField
                control={allocation.control}
                name="setup.adapter_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Profile</FormLabel>
                    <Select
                      value={field.value}
                      onValueChange={(v) => {
                        field.onChange(v);
                        allocation.setValue(
                          "setup.credential.catalog_service_id",
                          "",
                        );
                        allocation.setValue("setup.account_id", "");
                      }}
                    >
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="telnyx.messaging">
                          Telnyx SMS
                        </SelectItem>
                        <SelectItem value="twilio.messaging">
                          Twilio SMS
                        </SelectItem>
                        <SelectItem value="twilio.conversation_relay">
                          Twilio ConversationRelay
                        </SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={allocation.control}
                name="setup.account_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Trusted account</FormLabel>
                    <Select
                      value={field.value}
                      onValueChange={(id) => {
                        const a = data.accounts.data?.find(
                          (a) =>
                            a.account_id === id &&
                            a.provider === allocationProfile.split(".")[0],
                        );
                        field.onChange(id);
                        allocation.setValue(
                          "setup.credential.catalog_service_id",
                          a?.catalog_service_id ?? "",
                        );
                      }}
                    >
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder="Choose trusted account" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        {data.accounts.data
                          ?.filter(
                            (a) =>
                              a.provider === allocationProfile.split(".")[0],
                          )
                          .map((a) => (
                            <SelectItem key={a.id} value={a.account_id}>
                              {a.account_id}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <div>
                <label htmlFor="allocation-owner-search">
                  Find owner by email
                </label>
                <Input
                  id="allocation-owner-search"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                />
              </div>
              <FormField
                control={allocation.control}
                name="owner_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Owner</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder="Choose a person" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        {users.data?.users
                          .filter((u) => u.is_active)
                          .map((u) => (
                            <SelectItem key={u.id} value={u.id}>
                              {u.email}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              {(
                [
                  ["setup.phone_number", "Phone number (+country code)"],
                  ["setup.resource_id", "Provider number resource ID"],
                  [
                    "setup.application_id",
                    "Provider application / messaging profile ID",
                  ],
                  ["tariff_version", "Tariff version"],
                  ["tariff_source", "Official tariff evidence URL"],
                ] as const
              ).map(([name, label]) => (
                <FormField
                  key={name}
                  control={allocation.control}
                  name={name}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{label}</FormLabel>
                      <FormControl>
                        <Input {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ))}
              <FormField
                control={allocation.control}
                name="includes_all_provider_charges"
                render={({ field }) => (
                  <FormItem className="flex items-center gap-2">
                    <FormControl>
                      <Checkbox
                        checked={field.value}
                        onCheckedChange={(value) =>
                          allocation.setValue(
                            "includes_all_provider_charges",
                            value === true,
                          )
                        }
                      />
                    </FormControl>
                    <FormLabel>
                      The synchronized catalog price covers all carrier, relay
                      and selected speech charges.
                    </FormLabel>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <p className="text-muted-foreground">
                NyxID checks actual provider number/application ownership.
                Configure a synchronized Requests price per SMS segment or Voice
                seconds price for calls in Billing. Components and
                operation-specific prices are currently unavailable for these
                profiles. Tariff evidence must be refreshed within 30 days.
              </p>
              <Button
                type="submit"
                disabled={
                  !allocation.formState.isDirty ||
                  !allocation.formState.isValid ||
                  data.allocate.isPending
                }
              >
                Verify and allocate
              </Button>
            </form>
          </Form>
        </details>
      </div>
      {error && <ErrorBanner message={error} />}
      <Dialog
        open={!!remove}
        onOpenChange={(open) => {
          if (!open) setRemove(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {remove?.kind === "account"
                ? "Remove account trust"
                : "Revoke phone allocation"}
            </DialogTitle>
            <DialogDescription>
              {remove?.kind === "account"
                ? "All allocations must be removed first. Existing bindings lose verification authority."
                : "This disables the owner’s bindings and removes them once all sessions and admitted usage are finalized. A pending cleanup returns a conflict; retry after it completes. The resource can then be assigned to another owner."}
            </DialogDescription>
          </DialogHeader>
          <p>{remove?.name}</p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemove(undefined)}>
              Cancel
            </Button>
            <Button
              disabled={data.revoke.isPending || data.removeAccount.isPending}
              onClick={() =>
                void act(async () => {
                  if (!remove) return;
                  await (
                    remove.kind === "account" ? data.removeAccount : data.revoke
                  ).mutateAsync(remove.id);
                  setRemove(undefined);
                })
              }
            >
              Confirm removal
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
