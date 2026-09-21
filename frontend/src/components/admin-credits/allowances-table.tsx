import { MoreHorizontal } from "lucide-react";
import { billingTargetLabel } from "@/lib/billing-targets";
import { billingMetricLabel } from "@/lib/billing-units";
import type { DownstreamService } from "@/types/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { bundleStatus, type AllowanceBundle } from "./allowance-bundles";

export function AllowancesTable({
  bundles,
  services,
  canWrite,
  pending,
  onEdit,
  onToggle,
}: {
  bundles: AllowanceBundle[];
  services: readonly DownstreamService[];
  canWrite: boolean;
  pending: boolean;
  onEdit: (bundle: AllowanceBundle) => void;
  onToggle: (bundle: AllowanceBundle) => void;
}) {
  const name = (bundle: AllowanceBundle) =>
    services.find((s) => s.id === bundle.service_id)?.name ??
    bundle.service_slug;
  const units = (bundle: AllowanceBundle) => (
    <div className="space-y-1">
      {bundle.rows.map((row) => (
        <div key={row.id}>
          {row.quantity.toLocaleString()}{" "}
          {billingMetricLabel(row.metric, row.quantity)} ·{" "}
          {row.recurrence.replaceAll("_", " ")}
          {!row.is_active && (
            <span className="ml-2 text-muted-foreground">Disabled</span>
          )}
        </div>
      ))}
    </div>
  );
  const status = (bundle: AllowanceBundle) => (
    <Badge
      variant={
        bundleStatus(bundle) === "Active"
          ? "success"
          : bundle.is_active
            ? "warning"
            : "secondary"
      }
    >
      {bundleStatus(bundle)}
    </Badge>
  );
  const actions = (bundle: AllowanceBundle) =>
    canWrite && (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Actions for ${bundle.service_slug}`}
          >
            <MoreHorizontal className="size-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => onEdit(bundle)}>
            Edit
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={pending}
            onSelect={() => onToggle(bundle)}
          >
            {bundle.is_active ? "Disable" : "Enable"}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
  if (!bundles.length)
    return (
      <p className="py-10 text-center text-[12px] text-muted-foreground">
        No usage allowances.
      </p>
    );
  return (
    <>
      <div className="flex flex-col gap-3 md:hidden">
        {bundles.map((bundle) => (
          <div
            key={bundle.id}
            className="relative rounded-xl border border-border/50 bg-card p-4"
          >
            <div className="absolute right-2 top-2">{actions(bundle)}</div>
            <div className="pr-8 text-[13px] font-semibold">{name(bundle)}</div>
            <div className="my-2 text-[11px]">{units(bundle)}</div>
            <div className="mb-2 text-[11px] text-muted-foreground">
              {billingTargetLabel(bundle)}
            </div>
            {status(bundle)}
          </div>
        ))}
      </div>
      <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Service</TableHead>
              <TableHead>Units</TableHead>
              <TableHead>Targets</TableHead>
              <TableHead>Status</TableHead>
              {canWrite && (
                <TableHead className="text-right">Actions</TableHead>
              )}
            </TableRow>
          </TableHeader>
          <TableBody>
            {bundles.map((bundle) => (
              <TableRow key={bundle.id}>
                <TableCell>
                  <div className="font-medium">{name(bundle)}</div>
                  <div className="text-[11px] text-muted-foreground">
                    {bundle.service_slug}
                  </div>
                </TableCell>
                <TableCell>{units(bundle)}</TableCell>
                <TableCell>{billingTargetLabel(bundle)}</TableCell>
                <TableCell>{status(bundle)}</TableCell>
                {canWrite && (
                  <TableCell className="text-right">
                    {actions(bundle)}
                  </TableCell>
                )}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </>
  );
}
