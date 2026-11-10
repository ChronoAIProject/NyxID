import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useState } from "react";
import { useUpdateApiKey } from "@/hooks/use-api-keys";
import { ApiError } from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Gauge, Pencil } from "lucide-react";
import { toast } from "sonner";

export function RateLimitCard({
  keyId,
  rateLimitPerSecond,
  rateLimitBurst,
}: {
  readonly keyId: string;
  readonly rateLimitPerSecond: number | null;
  readonly rateLimitBurst: number | null;
}) {
  const { t } = useUiTranslation();
  const [editing, setEditing] = useState(false);
  const [rps, setRps] = useState(rateLimitPerSecond?.toString() ?? "");
  const [burst, setBurst] = useState(rateLimitBurst?.toString() ?? "");
  const updateApiKey = useUpdateApiKey();

  function handleSave() {
    const rpsNum = rps.trim() ? Number(rps) : null;
    const burstNum = burst.trim() ? Number(burst) : null;

    if (
      rpsNum !== null &&
      (!Number.isInteger(rpsNum) || rpsNum < 1 || rpsNum > 10000)
    ) {
      toast.error(translateUi("Rate limit per second must be an integer between 1 and 10000"));
      return;
    }
    if (
      burstNum !== null &&
      (!Number.isInteger(burstNum) || burstNum < 1 || burstNum > 100000)
    ) {
      toast.error(translateUi("Burst limit must be an integer between 1 and 100000"));
      return;
    }

    updateApiKey.mutate(
      {
        keyId,
        rate_limit_per_second: rpsNum,
        rate_limit_burst: burstNum,
      },
      {
        onSuccess: () => {
          toast.success(translateUi("Rate limits updated"));
          setEditing(false);
        },
        onError: (err) => {
          const message =
            err instanceof ApiError
              ? err.message
              : "Failed to update rate limits";
          toast.error(translateErrorMessage(message));
        },
      },
    );
  }

  function handleCancel() {
    setRps(rateLimitPerSecond?.toString() ?? "");
    setBurst(rateLimitBurst?.toString() ?? "");
    setEditing(false);
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center gap-2">
          <Gauge className="h-4 w-4 text-primary-text" />
          <CardTitle className="text-15">{t("Rate Limits")}</CardTitle>
        </div>
        <CardDescription>{t("Per-agent request rate limits (overrides user-level defaults)")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {editing ? (
          <div className="space-y-3">
            <div className="space-y-1.5">
              <Label className="text-xs">{t("Requests per second")}</Label>
              <Input
                type="number"
                min={1}
                value={rps}
                onChange={(e) => setRps(e.target.value)}
                placeholder={t("Use user default")}
              />
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs">{t("Burst limit")}</Label>
              <Input
                type="number"
                min={1}
                value={burst}
                onChange={(e) => setBurst(e.target.value)}
                placeholder={t("Use user default")}
              />
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={handleCancel}>{t("Cancel")}</Button>
              <Button
                variant="primary"
                onClick={handleSave}
                disabled={updateApiKey.isPending || (rps === (rateLimitPerSecond?.toString() ?? "") && burst === (rateLimitBurst?.toString() ?? ""))}
              >{t("Save")}</Button>
            </div>
          </div>
        ) : (
          <div className="flex items-center justify-between">
            <div className="grid grid-cols-2 gap-2 text-xs">
              <div>
                <span className="text-muted-foreground">{t("Per second: ")}</span>
                {rateLimitPerSecond != null
                  ? String(rateLimitPerSecond)
                  : t("User default")}
              </div>
              <div>
                <span className="text-muted-foreground">{t("Burst: ")}</span>
                {rateLimitBurst != null
                  ? String(rateLimitBurst)
                  : t("User default")}
              </div>
            </div>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6 shrink-0"
              onClick={() => setEditing(true)}
            >
              <Pencil className="h-3 w-3" />
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
