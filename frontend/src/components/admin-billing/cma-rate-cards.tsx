import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useCmaRateCards,
  usePublishCmaRateCard,
} from "@/hooks/use-cma-rate-cards";
import {
  CMA_RESOURCE_LABELS,
  cmaRateFormSchema,
  type CmaRateCard,
  type CmaRateForm,
} from "@/schemas/cma-rate-cards";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export function CmaRateCards({ canWrite }: { readonly canWrite: boolean }) {
  useUiTranslation();
  const cards = useCmaRateCards();
  return (
    <Card>
      <CardHeader>
        <CardTitle>{translateUi("CMA Sandbox rates")}</CardTitle>
        <p className="type-body text-muted-foreground">{translateUi("Prices apply to allocated CPU, RAM and retained disk. Publishing a rate does not enable billing. Use Feature Flags to enable CMA billing globally or for selected people.")}</p>
      </CardHeader>
      <CardContent className="space-y-4">
        {cards.error && (
          <ErrorBanner
            message={translateUi("Unable to load CMA rate history")}
            onRetry={() => void cards.refetch()}
          />
        )}
        {cards.isLoading && <p>{translateUi("Loading CMA rates…")}</p>}
        {cards.data?.length === 0 && (
          <p>{translateUi("The initial CMA rate card has not been installed.")}</p>
        )}
        {canWrite && cards.data?.[0] && (
          <PublishForm key={cards.data[0].version} latest={cards.data[0]} />
        )}
        {!!cards.data?.length && (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{translateUi("Version")}</TableHead>
                <TableHead>{translateUi("Effective from")}</TableHead>
                <TableHead>{translateUi("CPU / millicore-second")}</TableHead>
                <TableHead>{translateUi("RAM / MiB-second")}</TableHead>
                <TableHead>{translateUi("Disk / GiB-second")}</TableHead>
                <TableHead>{translateUi("Price sync")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {cards.data.map((card) => (
                <TableRow key={card.id}>
                  <TableCell>{card.version}</TableCell>
                  <TableCell>
                    {new Date(card.effective_from).toLocaleString(getFormattingLocale())}
                  </TableCell>
                  {Object.keys(CMA_RESOURCE_LABELS).map((resource) => (
                    <TableCell key={resource}>
                      {card.rates[resource as keyof typeof CMA_RESOURCE_LABELS]}
                    </TableCell>
                  ))}
                  <TableCell>{card.sync_status}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}
function PublishForm({ latest }: { readonly latest: CmaRateCard }) {
  useUiTranslation();
  const publish = usePublishCmaRateCard();
  const form = useAppForm<CmaRateForm>({
    resolver: zodResolver(cmaRateFormSchema),
    defaultValues: { effective_from: "", rates: latest.rates },
  });
  const submit = (value: CmaRateForm) => {
    const effective = new Date(value.effective_from);
    if (
      effective.getTime() < Date.now() + 301_000 ||
      effective <= new Date(latest.effective_from)
    ) {
      form.setError("effective_from", {
        message:
          "Choose a time at least 301 seconds from now and after the latest version.",
      });
      return;
    }
    publish.mutate({
      expected_version: latest.version,
      effective_from: effective.toISOString(),
      rates: value.rates,
    });
  };
  return (
    <Form {...form}>
      <form
        onSubmit={(event) => {
          void form.handleSubmit(submit)(event);
        }}
        className="space-y-4"
      >
        <p className="type-body text-muted-foreground">{translateUi("Publish a new version with at least 301 seconds’ notice. Existing charges keep their original rates. Sandboxes need a fresh quote and authorized operation at the price change.")}</p>
        {Object.entries(CMA_RESOURCE_LABELS).map(([resource, label]) => (
          <FormField
            key={resource}
            control={form.control}
            name={`rates.${resource as keyof typeof CMA_RESOURCE_LABELS}`}
            render={({ field }) => (
              <FormItem>
                <FormLabel>{label}</FormLabel>
                <FormControl>
                  <Input {...field} inputMode="decimal" />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        ))}
        <FormField
          control={form.control}
          name="effective_from"
          render={({ field }) => (
            <FormItem>
              <FormLabel>{translateUi("Effective from (local time)")}</FormLabel>
              <FormControl>
                <Input {...field} type="datetime-local" step="1" />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        {publish.error && (
          <ErrorBanner
            message={
              publish.error instanceof Error
                ? publish.error.message
                : translateUi("Unable to publish rate card")
            }
          />
        )}
        {publish.isSuccess && (
          <p role="status">{translateUi("Rate card published. Price synchronization is pending.")}</p>
        )}
        <Button type="submit" disabled={publish.isPending}>{translateUi("Publish rate version")}</Button>
      </form>
    </Form>
  );
}
