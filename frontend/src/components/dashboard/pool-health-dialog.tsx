import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useState } from "react";
import { toast } from "sonner";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { usePoolHealth, useResetPoolHealth } from "@/hooks/use-pools";
import type { ServicePool } from "@/schemas/pools";
import {
  bindingLabel,
  message,
  readOnlyPreview,
  reason,
  strategyLabels,
} from "./pool-labels";
import { PoolOperationCheck, type PoolOperation } from "./pool-operation-check";

export function PoolHealthDialog({
  pool,
  onClose,
  onCloseAutoFocus,
}: {
  pool: ServicePool;
  onClose: () => void;
  onCloseAutoFocus?: (event: Event) => void;
}) {
  const { t } = useUiTranslation();
  const aiChat = pool.member_contract === "ai_chat";
  const [operation, setOperation] = useState<PoolOperation | null>(null);
  const health = usePoolHealth({
    poolId: pool.id,
    checkOperation: aiChat || operation !== null,
    method: aiChat ? "POST" : operation?.method,
    path: aiChat ? "chat/completions" : operation?.path,
    contract: pool.member_contract,
  });
  const reset = useResetPoolHealth();
  async function clear(userServiceId?: string) {
    try {
      await reset.mutateAsync({ poolId: pool.id, userServiceId });
      toast.success(translateUi("Cooldown reset"));
    } catch (error) {
      toast.error(translateErrorMessage(message(error)));
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent
        onCloseAutoFocus={onCloseAutoFocus}
        className="md:max-w-2xl [&_input:focus-visible]:border-primary [&_input:focus-visible]:ring-1 [&_input:focus-visible]:ring-primary/40"
      >
        <DialogHeader>
          <DialogTitle className="break-words">{pool.name}{t(" health")}</DialogTitle>
          <DialogDescription>{t("Availability and recent failures for your saved connections. Cooldowns depend on the operation being called. Reset lets a connection be tried again; it does not test the provider.")}</DialogDescription>
        </DialogHeader>
        {aiChat ? (
          <p className="text-12 text-muted-foreground">{t("Checking AI chat requests (POST /chat/completions).")}</p>
        ) : (
          <PoolOperationCheck operation={operation} onChange={setOperation} />
        )}
        {!aiChat && !operation && (
          <p className="text-12 text-muted-foreground">{t("Choose an operation above to see its cooldown and failure history. Connection availability is shown below.")}</p>
        )}
        {pool.strategy !== "priority" && (
          <p className="text-12 text-muted-foreground">
            {strategyLabels[pool.strategy]}{t(" sends one attempt per request and does not use cooldowns.")}</p>
        )}
        {health.isError && <ErrorBanner message={message(health.error)} />}
        {health.isLoading && <Skeleton className="h-16" />}
        <div className="space-y-2">
          {health.data?.candidates.length === 0 && (
            <p className="text-12 text-muted-foreground">{t("This pool has no connections yet. Edit the pool to add them.")}</p>
          )}
          {health.data?.candidates.map((row) => (
            <div
              key={row.user_service_id}
              className="flex items-center justify-between gap-3 rounded-xl border border-border/50 p-3"
            >
              <div className="min-w-0 flex-1">
                <div className="break-words text-12 font-medium">
                  {row.name || row.slug}{" "}
                  <Badge variant={row.eligible ? "success" : "warning"}>
                    {row.reason
                      ? reason(row)
                      : health.data?.operation_checked
                        ? t("Ready for this operation")
                        : t("Available connection")}
                  </Badge>
                </div>
                <p className="break-words text-11 text-muted-foreground">
                  {row.slug} · {bindingLabel(row.credential_binding)}
                  {pool.strategy === "priority"
                    ? t(" · Priority {{value}}", { value: pool.members.find((m) => m.user_service_id === row.user_service_id)?.priority ?? 0 })
                    : ""}
                  {pool.members.find(
                    (m) => m.user_service_id === row.user_service_id,
                  )?.model
                    ? t(" · {{model}}", { model: pool.members.find((m) => m.user_service_id === row.user_service_id)?.model })
                    : ""}
                  {health.data?.operation_checked &&
                  pool.strategy === "priority"
                    ? t(" · {{consecutive_failures}} failures", { consecutive_failures: row.consecutive_failures })
                    : ""}
                  {row.last_status ? t(" · HTTP {{last_status}}", { last_status: row.last_status }) : ""}
                  {row.cooldown_until
                    ? t(" · Retry after {{value}}", { value: new Date(row.cooldown_until).toLocaleString(getFormattingLocale()) })
                    : ""}
                </p>
              </div>
              {pool.strategy === "priority" && (
                <Button
                  className="shrink-0"
                  onClick={() => {
                    void clear(row.user_service_id);
                  }}
                  disabled={readOnlyPreview || reset.isPending}
                >{t("Reset")}</Button>
              )}
            </div>
          ))}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>{t("Close")}</Button>
          {pool.strategy === "priority" && pool.members.length > 0 && (
            <Button
              onClick={() => {
                void clear();
              }}
              isLoading={reset.isPending}
              disabled={readOnlyPreview}
            >{t("Reset all cooldowns")}</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
