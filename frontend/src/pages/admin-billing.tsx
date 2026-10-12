import { Fragment, useEffect, useMemo, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  EffectiveBilling,
  SourceBadge,
} from "@/components/admin-billing/effective-billing";
import {
  BillingLaneFields,
  type BillingLaneFormData,
} from "@/components/services/billing-lane-fields";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  useAdminBillingConfig,
  useAdminBillingOverview,
  useAdminBillingRates,
  useResyncAdminBillingService,
  useUpdateAdminBillingConfig,
  useUpdateAdminBillingService,
} from "@/hooks/use-admin-billing";
import { useService } from "@/hooks/use-services";
import {
  ADMIN_BILLING_TABS,
  ADMIN_BILLING_TAB_DEFAULT,
  parseTab,
} from "@/lib/url-tabs";
import {
  billingSettingsFormSchema,
  billingSettingsPatch,
  type BillingConfig,
  type BillingService,
  type BillingSettingsForm,
} from "@/schemas/admin-billing";
import { sharedServiceSchema } from "@/schemas/services";
import {
  serviceLanePatch,
  serviceLaneValues,
} from "@/pages/service-edit.helpers";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";

function errorText(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function formatTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export function AdminBillingPage() {
  const search = useSearch({ strict: false });
  const navigate = useNavigate();
  const tab = parseTab(
    search.tab,
    ADMIN_BILLING_TABS,
    ADMIN_BILLING_TAB_DEFAULT,
  );
  const canWrite = canAdminWrite(useAuthStore((state) => state.user));
  return (
    <div className="space-y-6">
      <PageHeader
        title="Billing"
        description="Every billed service's effective prices with their source, the Lago rate cache, and global defaults."
      />
      <Tabs
        value={tab}
        onValueChange={(value) => {
          void navigate({
            to: "/admin/billing",
            search: { tab: value },
            replace: true,
          });
        }}
      >
        <TabsList>
          <TabsTrigger value="overview">Services</TabsTrigger>
          <TabsTrigger value="rates">Rate cache</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="space-y-4">
          <ServicesTab canWrite={canWrite} />
        </TabsContent>
        <TabsContent value="rates" className="space-y-4">
          <RatesTab />
        </TabsContent>
        <TabsContent value="settings" className="space-y-4">
          <SettingsTab canWrite={canWrite} />
        </TabsContent>
      </Tabs>
    </div>
  );
}

function ServicesTab({ canWrite }: { readonly canWrite: boolean }) {
  const overview = useAdminBillingOverview();
  const resync = useResyncAdminBillingService();
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const rows = useMemo(
    () =>
      (overview.data ?? []).filter((service) =>
        `${service.name} ${service.slug}`
          .toLowerCase()
          .includes(query.trim().toLowerCase()),
      ),
    [overview.data, query],
  );
  const columns = canWrite ? 3 : 2;
  return (
    <Card>
      <CardHeader className="space-y-3">
        <CardTitle>Billed services</CardTitle>
        <p className="text-12 text-muted-foreground">
          Precedence per credential class: a synced lane price, else legacy
          platform billing while a lane is unsynced or absent, else free.
          BILLING_ENABLED and BILLING_RESALE_ENABLED override everything.
        </p>
        <Input
          aria-label="Search services"
          placeholder="Search services"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </CardHeader>
      <CardContent>
        {overview.isLoading && <Skeleton className="h-24 w-full" />}
        {overview.error && (
          <ErrorBanner
            message={errorText(
              overview.error,
              "Failed to load billed services",
            )}
            onRetry={() => void overview.refetch()}
          />
        )}
        {overview.data && rows.length === 0 && (
          <p className="text-12 text-muted-foreground">
            {overview.data.length === 0
              ? "No catalog service has billing configuration."
              : "No service matches the search."}
          </p>
        )}
        {rows.length > 0 && (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Service</TableHead>
                <TableHead>Effective billing by credential</TableHead>
                {canWrite && (
                  <TableHead className="text-right">Actions</TableHead>
                )}
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((service) => (
                <Fragment key={service.id}>
                  <TableRow>
                    <TableCell className="align-top">
                      <div className="font-medium">{service.name}</div>
                      <div className="text-11 text-muted-foreground">
                        {service.slug}
                      </div>
                      {!service.is_active && (
                        <Badge variant="secondary">Inactive</Badge>
                      )}
                    </TableCell>
                    <TableCell className="align-top">
                      <EffectiveBilling service={service} />
                    </TableCell>
                    {canWrite && (
                      <TableCell className="space-y-2 text-right align-top">
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() =>
                            setEditing(
                              editing === service.id ? null : service.id,
                            )
                          }
                        >
                          {editing === service.id
                            ? "Close editor"
                            : "Edit prices"}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={resync.isPending}
                          onClick={() =>
                            resync.mutate(service.id, {
                              onSuccess: (result) =>
                                result.synced
                                  ? toast.success(
                                      `${service.name} prices synced to Lago`,
                                    )
                                  : toast.warning(
                                      `${service.name}: nothing synced; check the sync errors`,
                                    ),
                              onError: (error) =>
                                toast.error(errorText(error, "Resync failed")),
                            })
                          }
                        >
                          Resync
                        </Button>
                      </TableCell>
                    )}
                  </TableRow>
                  {canWrite && editing === service.id && (
                    <TableRow>
                      <TableCell colSpan={columns}>
                        <ServiceLaneEditor
                          service={service}
                          onDone={() => setEditing(null)}
                        />
                      </TableCell>
                    </TableRow>
                  )}
                </Fragment>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}

const laneFormSchema = sharedServiceSchema.pick({
  inference: true,
  byok_pricing: true,
  platform_key_pricing: true,
});

function ServiceLaneEditor({
  service,
  onDone,
}: {
  readonly service: BillingService;
  readonly onDone: () => void;
}) {
  const detail = useService(service.id);
  if (detail.isLoading) return <Skeleton className="h-24 w-full" />;
  if (detail.error || !detail.data)
    return (
      <ErrorBanner
        message={errorText(detail.error, "Failed to load the service")}
        onRetry={() => void detail.refetch()}
      />
    );
  return <LaneForm service={detail.data} name={service.name} onDone={onDone} />;
}

function LaneForm({
  service,
  name,
  onDone,
}: {
  readonly service: NonNullable<ReturnType<typeof useService>["data"]>;
  readonly name: string;
  readonly onDone: () => void;
}) {
  const update = useUpdateAdminBillingService();
  const form = useAppForm<BillingLaneFormData>({
    defaultValues: {
      inference: service.inference ?? null,
      ...serviceLaneValues(service),
    },
    resolver: zodResolver(laneFormSchema),
  });
  const onSubmit = form.handleSubmit((data) => {
    const billing = serviceLanePatch(service, data);
    if (Object.keys(billing).length === 0) {
      onDone();
      return;
    }
    update.mutate(
      { serviceId: service.id, billing },
      {
        onSuccess: () => {
          toast.success(`${name} prices saved`);
          onDone();
        },
        onError: (error) =>
          toast.error(errorText(error, "Failed to save prices")),
      },
    );
  });
  return (
    <Form {...form}>
      <form onSubmit={(event) => void onSubmit(event)} className="space-y-3">
        <BillingLaneFields service={service} />
        {update.error && (
          <p role="alert" className="text-12 text-destructive">
            {errorText(update.error, "Failed to save prices")}
          </p>
        )}
        <div className="flex gap-2">
          <Button
            type="submit"
            disabled={!form.formState.isDirty || update.isPending}
          >
            Save prices
          </Button>
          <Button type="button" variant="outline" onClick={onDone}>
            Cancel
          </Button>
        </div>
      </form>
    </Form>
  );
}

function RatesTab() {
  const rates = useAdminBillingRates();
  return (
    <Card>
      <CardHeader>
        <CardTitle>Lago rate cache</CardTitle>
      </CardHeader>
      <CardContent>
        {rates.isLoading && <Skeleton className="h-24 w-full" />}
        {rates.error && (
          <ErrorBanner
            message={errorText(rates.error, "Failed to load rates")}
            onRetry={() => void rates.refetch()}
          />
        )}
        {rates.data?.length === 0 && (
          <p className="text-12 text-muted-foreground">
            No rates are cached yet.
          </p>
        )}
        {rates.data && rates.data.length > 0 && (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Metric code</TableHead>
                <TableHead>Model</TableHead>
                <TableHead>Credits per unit</TableHead>
                <TableHead>Synced</TableHead>
                <TableHead>Stale after</TableHead>
                <TableHead>State</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rates.data.map((rate) => (
                <TableRow key={rate.id}>
                  <TableCell className="font-mono text-12">
                    {rate.lago_metric_code}
                  </TableCell>
                  <TableCell>{rate.model ?? "Any"}</TableCell>
                  <TableCell>{rate.credits_per_unit}</TableCell>
                  <TableCell>{formatTime(rate.synced_at)}</TableCell>
                  <TableCell>{formatTime(rate.stale_after)}</TableCell>
                  <TableCell className="space-x-1">
                    {rate.retired_at ? (
                      <Badge variant="destructive">Retired</Badge>
                    ) : rate.stale ? (
                      <Badge variant="warning">Stale</Badge>
                    ) : (
                      <Badge variant="success">Current</Badge>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}

function SettingsTab({ canWrite }: { readonly canWrite: boolean }) {
  const config = useAdminBillingConfig();
  if (config.isLoading) return <Skeleton className="h-40 w-full" />;
  if (config.error || !config.data)
    return (
      <ErrorBanner
        message={errorText(
          config.error,
          "Failed to load billing configuration",
        )}
        onRetry={() => void config.refetch()}
      />
    );
  const data = config.data;
  const cap = data.default_overdraft_cap_credits;
  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Global defaults</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex flex-wrap items-center gap-2 text-13">
            <span>
              New-wallet overdraft cap: <strong>{cap.effective}</strong> credits
            </span>
            <SourceBadge source={cap.source} />
          </div>
          <p className="text-12 text-muted-foreground">
            Applies when a wallet is first provisioned (first charge, top-up
            checkout, or backfill). Existing wallets keep their cap. Empty uses
            BILLING_DEFAULT_OVERDRAFT_CAP_CREDITS ({cap.env}).
          </p>
          {canWrite && <SettingsForm config={data} />}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Deployment environment (read-only)</CardTitle>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Variable</TableHead>
                <TableHead>Value</TableHead>
                <TableHead>Source</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.environment.map((row) => (
                <TableRow key={row.key}>
                  <TableCell className="font-mono text-12">{row.key}</TableCell>
                  <TableCell>
                    {row.secret_set === null
                      ? (row.value ?? "Not set")
                      : row.secret_set
                        ? "Set (hidden)"
                        : "Not set"}
                  </TableCell>
                  <TableCell>
                    <SourceBadge source={row.source} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}

function SettingsForm({ config }: { readonly config: BillingConfig }) {
  const update = useUpdateAdminBillingConfig();
  const stored = config.default_overdraft_cap_credits.global;
  const defaults: BillingSettingsForm = {
    default_overdraft_cap_credits: stored === null ? "" : String(stored),
  };
  const form = useAppForm<BillingSettingsForm>({
    defaultValues: defaults,
    resolver: zodResolver(
      billingSettingsFormSchema(config.max_default_overdraft_cap_credits),
    ),
  });
  useEffect(() => {
    form.reset({
      default_overdraft_cap_credits: stored === null ? "" : String(stored),
    });
  }, [form, stored]);
  const onSubmit = form.handleSubmit((data) =>
    update.mutate(billingSettingsPatch(data), {
      onSuccess: () => toast.success("Global billing defaults saved"),
      onError: (error) =>
        toast.error(errorText(error, "Failed to save defaults")),
    }),
  );
  return (
    <Form {...form}>
      <form onSubmit={(event) => void onSubmit(event)} className="space-y-3">
        <FormField
          control={form.control}
          name="default_overdraft_cap_credits"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Global overdraft cap (credits)</FormLabel>
              <FormControl>
                <Input
                  {...field}
                  inputMode="numeric"
                  placeholder={`Environment default (${config.default_overdraft_cap_credits.env})`}
                />
              </FormControl>
              <FormDescription>
                Leave empty to use the environment default.
              </FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        {update.error && (
          <p role="alert" className="text-12 text-destructive">
            {errorText(update.error, "Failed to save defaults")}
          </p>
        )}
        <Button
          type="submit"
          disabled={!form.formState.isDirty || update.isPending}
        >
          Save global defaults
        </Button>
      </form>
    </Form>
  );
}
