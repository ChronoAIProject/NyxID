import { useState } from "react";
import { toast } from "sonner";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { credits, number, periods, timestamp } from "@/lib/billing-display";
import type { BillingUsagePeriod } from "@/schemas/billing";
import { useTopUpHistory, openInvoiceReceipt } from "@/hooks/use-billing";
import { openExternal } from "@/lib/navigation";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Skeleton } from "@/components/ui/skeleton";
const date = (value: string) => new Date(value).toLocaleDateString();

export function BillingTopUpHistory() {
  const [period, setPeriod] = useState<BillingUsagePeriod>("30d");
  return <TopUpHistory key={period} period={period} onPeriod={setPeriod} />;
}

function TopUpHistory({
  period,
  onPeriod,
}: {
  period: BillingUsagePeriod;
  onPeriod: (period: BillingUsagePeriod) => void;
}) {
  const [page, setPage] = useState(1);
  const historyQuery = useTopUpHistory(page, 10, period);
  const visible = historyQuery.data?.topups ?? [];
  const total = historyQuery.data?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / 10));
  const currentPage = page;
  async function handleReceipt(invoiceId: string) {
    try {
      await openInvoiceReceipt(invoiceId);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "The receipt is not ready yet; try again shortly",
      );
    }
  }
  if (historyQuery.isLoading) return <Skeleton className="h-[200px] w-full" />;
  if (historyQuery.isError)
    return (
      <ErrorBanner
        message="Failed to load top-up history."
        onRetry={() => void historyQuery.refetch()}
      />
    );
  const expiry = (payment: (typeof visible)[number]) =>
    payment.credits_expired_at
      ? `${date(payment.credits_expired_at)} · ${credits(payment.expired_credits_micros)} credits expired`
      : payment.credits_expire_at
        ? date(payment.credits_expire_at)
        : payment.status === "paid"
          ? "Pending sync"
          : "—";
  const action = (payment: (typeof visible)[number]) =>
    payment.status === "pending" && !!payment.checkout_url ? (
      <Button
        variant="outline"
        size="sm"
        onClick={() => openExternal(payment.checkout_url!)}
      >
        Resume payment
      </Button>
    ) : payment.receipt_available && !!payment.lago_invoice_id ? (
      <Button
        variant="outline"
        size="sm"
        onClick={() => void handleReceipt(payment.lago_invoice_id!)}
      >
        Download receipt
      </Button>
    ) : (
      <span className="text-muted-foreground">—</span>
    );
  const status = (payment: (typeof visible)[number]) => (
    <Badge
      variant={
        payment.status === "paid"
          ? "success"
          : payment.status === "expired"
            ? "warning"
            : payment.status === "failed"
              ? "destructive"
              : "secondary"
      }
      className="capitalize"
    >
      {payment.status}
    </Badge>
  );
  return (
    <Card className="topup-history-card">
      <CardHeader className="history-heading">
        <div>
          <CardTitle>Top-up history</CardTitle>
          <p className="mt-1 text-[12px] text-muted-foreground">
            Payments, their status, and downloadable receipts.
          </p>
        </div>
        <Select
          value={period}
          onValueChange={(value) => {
            onPeriod(value as BillingUsagePeriod);
            setPage(1);
          }}
        >
          <SelectTrigger
            aria-label="Top-up history period"
            className="w-[148px]"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {Object.entries(periods).map(([key, label]) => (
              <SelectItem key={key} value={key}>
                {label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </CardHeader>
      <CardContent>
        <div className="topup-desktop overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Date</TableHead>
                <TableHead className="text-right">Credits</TableHead>
                <TableHead>Credit expiry</TableHead>
                <TableHead>Invoice</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visible.length ? (
                visible.map((payment) => (
                  <TableRow key={payment.id}>
                    <TableCell>{timestamp(payment.created_at)}</TableCell>
                    <TableCell className="text-right">
                      {number(payment.amount_credits)}
                    </TableCell>
                    <TableCell>{expiry(payment)}</TableCell>
                    <TableCell>{payment.invoice_number ?? "—"}</TableCell>
                    <TableCell>{status(payment)}</TableCell>
                    <TableCell className="text-right">
                      {action(payment)}
                    </TableCell>
                  </TableRow>
                ))
              ) : (
                <TableRow>
                  <TableCell
                    colSpan={6}
                    className="py-8 text-center text-muted-foreground"
                  >
                    No top-ups yet.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>
        <div className="topup-mobile">
          {visible.length ? (
            visible.map((payment) => (
              <div key={payment.id} className="mobile-topup">
                <div>
                  <strong>{number(payment.amount_credits)} credits</strong>
                  {status(payment)}
                </div>
                <dl className="split-facts">
                  <div>
                    <dt>Date</dt>
                    <dd>{timestamp(payment.created_at)}</dd>
                  </div>
                  <div>
                    <dt>Invoice</dt>
                    <dd>{payment.invoice_number ?? "—"}</dd>
                  </div>
                  <div>
                    <dt>Credit expiry</dt>
                    <dd>{expiry(payment)}</dd>
                  </div>
                </dl>
                {action(payment)}
              </div>
            ))
          ) : (
            <p className="py-8 text-center text-[12px] text-muted-foreground">
              No top-ups yet.
            </p>
          )}
        </div>
        {pageCount > 1 && (
          <div className="payments-controls mt-3">
            <span>
              Page {currentPage} of {pageCount}
            </span>
            <div>
              <Button
                variant="outline"
                size="sm"
                disabled={currentPage === 1 || historyQuery.isFetching}
                onClick={() => setPage(currentPage - 1)}
              >
                Previous
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={currentPage === pageCount || historyQuery.isFetching}
                onClick={() => setPage(currentPage + 1)}
              >
                Next
              </Button>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
