import { useState } from "react";
import { toast } from "sonner";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  BILLING_INVOICES_PAGE_SIZE,
  useCurrentBill,
  useBillingInvoices,
  openInvoiceReceipt,
} from "@/hooks/use-billing";
import { decimalCredits, parseCredits } from "@/lib/credits";
import type { UsageInvoice } from "@/schemas/billing";

const date = (value: string) =>
  new Date(value).toLocaleDateString(undefined, { timeZone: "UTC" });
const money = (amount: string, currency: string) =>
  new Intl.NumberFormat(undefined, {
    style: "currency",
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(Number(amount));
const percent = (value: string) => decimalCredits(parseCredits(value));
const PAYMENT_STATUS: Record<string, string> = {
  succeeded: "Paid",
  pending: "Pending",
  failed: "Failed",
};
const paymentStatus = (status: string) => PAYMENT_STATUS[status] ?? "Unknown";
const note =
  "Only wallet-funded usage appears on this bill. Usage covered by grants or allowances is excluded and is not taxed.";

export function BillingBill({ enabled }: { enabled: boolean }) {
  const [page, setPage] = useState(1);
  const bill = useCurrentBill(enabled);
  const currentBill = bill.data;
  const history = useBillingInvoices(page, enabled);
  const [downloading, setDownloading] = useState<string | null>(null);
  if (!enabled) return null;
  async function receipt(invoice: UsageInvoice) {
    setDownloading(invoice.id);
    try {
      await openInvoiceReceipt(invoice.id);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "The receipt could not be downloaded. Try again shortly.",
      );
    } finally {
      setDownloading(null);
    }
  }
  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Current bill</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4 text-12">
          {bill.isLoading ? (
            <Skeleton className="h-40 w-full" />
          ) : bill.isError ? (
            <ErrorBanner
              message="The current bill could not be loaded."
              onRetry={() => void bill.refetch()}
            />
          ) : (
            currentBill && (
              <>
                <p className="text-text-tertiary">
                  {date(currentBill.from)} – {date(currentBill.to)} ·{" "}
                  {currentBill.currency}
                </p>
                {currentBill.charges.length === 0 ? (
                  <p className="rounded-lg border border-dashed border-border p-4 text-muted-foreground">
                    No wallet-funded usage this billing period.
                  </p>
                ) : (
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Charge</TableHead>
                        <TableHead className="text-right">Units</TableHead>
                        <TableHead className="text-right">Amount</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {currentBill.charges.map((charge, index) => (
                        <TableRow key={`${charge.metric_code}:${index}`}>
                          <TableCell className="whitespace-normal">
                            {charge.name}
                          </TableCell>
                          <TableCell className="text-right font-mono tabular-nums">
                            {charge.units}
                          </TableCell>
                          <TableCell className="text-right font-mono tabular-nums">
                            {money(charge.amount, currentBill.currency)}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                )}
                <dl className="ml-auto max-w-sm space-y-2">
                  <Amount
                    label="Subtotal"
                    amount={currentBill.subtotal}
                    currency={currentBill.currency}
                  />
                  {currentBill.tax_breakdown_available &&
                    currentBill.taxes.map((tax) => (
                      <Amount
                        key={`${tax.code}:${tax.rate_percent}`}
                        label={`${tax.name} (${percent(tax.rate_percent)}%)`}
                        amount={tax.amount}
                        currency={currentBill.currency}
                      />
                    ))}
                  {currentBill.tax_breakdown_available &&
                    parseCredits(currentBill.rounding) !== 0n && (
                      <Amount
                        label="Tax rounding"
                        amount={currentBill.rounding}
                        currency={currentBill.currency}
                      />
                    )}
                  <Amount
                    label={
                      currentBill.tax_breakdown_available ? "Total tax" : "Tax"
                    }
                    amount={currentBill.tax_total}
                    currency={currentBill.currency}
                  />
                  <Amount
                    label="Total so far"
                    amount={currentBill.total}
                    currency={currentBill.currency}
                    strong
                  />
                </dl>
                <p className="text-text-tertiary">
                  This period is still open. Tax rows use your current tax
                  configuration; the total comes from your bill.
                </p>
              </>
            )
          )}
          <p className="text-text-tertiary">{note}</p>
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Past usage invoices</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4 text-12">
          {history.isLoading ? (
            <Skeleton className="h-32 w-full" />
          ) : history.isError ? (
            <ErrorBanner
              message="Usage invoices could not be loaded."
              onRetry={() => void history.refetch()}
            />
          ) : history.data?.invoices.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border p-4 text-muted-foreground">
              No finalized usage invoices yet. Your first invoice is issued at
              the end of the billing period.
            </p>
          ) : (
            history.data?.invoices.map((invoice) => (
              <article
                key={invoice.id}
                className="rounded-lg border border-border p-4"
              >
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div>
                    <p className="font-medium">{invoice.number}</p>
                    <p className="text-11 text-text-tertiary">
                      {date(invoice.issuing_date)} ·{" "}
                      {paymentStatus(invoice.payment_status)}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    isLoading={downloading === invoice.id}
                    disabled={downloading !== null}
                    onClick={() => void receipt(invoice)}
                  >
                    Download receipt
                  </Button>
                </div>
                <dl className="mt-3 space-y-2">
                  <Amount
                    label="Subtotal"
                    amount={invoice.subtotal}
                    currency={invoice.currency}
                  />
                  {invoice.taxes.map((tax) => (
                    <Amount
                      key={`${tax.code}:${tax.rate_percent}`}
                      label={`${tax.name} (${percent(tax.rate_percent)}%)`}
                      amount={tax.amount}
                      currency={invoice.currency}
                    />
                  ))}
                  <Amount
                    label="Total tax"
                    amount={invoice.tax_total}
                    currency={invoice.currency}
                  />
                  <Amount
                    label="Invoice total"
                    amount={invoice.total}
                    currency={invoice.currency}
                    strong
                  />
                  <Amount
                    label="Prepaid credits applied"
                    amount={invoice.prepaid_credits_applied}
                    currency={invoice.currency}
                  />
                  <Amount
                    label="Amount due"
                    amount={invoice.amount_due}
                    currency={invoice.currency}
                  />
                </dl>
              </article>
            ))
          )}
          {history.data && history.data.total > BILLING_INVOICES_PAGE_SIZE && (
            <div className="flex items-center justify-end gap-3">
              <span className="text-11 text-text-tertiary">
                Page {page} of{" "}
                {Math.ceil(history.data.total / BILLING_INVOICES_PAGE_SIZE)}
              </span>
              <Button
                size="sm"
                variant="outline"
                disabled={page === 1 || history.isFetching}
                onClick={() => setPage((p) => p - 1)}
              >
                Previous
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={
                  page * BILLING_INVOICES_PAGE_SIZE >= history.data.total ||
                  history.isFetching
                }
                onClick={() => setPage((p) => p + 1)}
              >
                Next
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </>
  );
}
function Amount({
  label,
  amount,
  currency,
  strong = false,
}: {
  label: string;
  amount: string;
  currency: string;
  strong?: boolean;
}) {
  return (
    <div
      className={`flex justify-between gap-4 ${strong ? "border-t border-border pt-2 font-semibold" : ""}`}
    >
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="font-mono tabular-nums">{money(amount, currency)}</dd>
    </div>
  );
}
