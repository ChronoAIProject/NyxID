import type { UseFormReturn } from "react-hook-form";
import type { UpdateServiceFormData } from "@/schemas/services";
import type { DownstreamService } from "@/types/api";
import { UserPicker } from "@/components/admin-credits/credit-pickers";
import {
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Label } from "@/components/ui/label";
import type {
  InferenceMetadata,
  PlatformKeyConfig,
  LanePricingView,
} from "@/schemas/platform-keys";

export function PlatformServiceFields({
  form,
  service,
}: {
  readonly form: UseFormReturn<UpdateServiceFormData>;
  readonly service: DownstreamService;
}) {
  const inference = form.watch("inference");
  const platform = form.watch("platform_key") ?? {
    enabled: false,
    audience: "restricted",
    allowed_owner_ids: [],
  };
  function setPlatform(values: Partial<PlatformKeyConfig>) {
    form.setValue(
      "platform_key",
      { ...platform, ...values },
      { shouldDirty: true, shouldValidate: true },
    );
  }
  return (
    <div className="space-y-5">
      <section className="space-y-3">
        <h3 className="text-[13px] font-semibold">Inference</h3>
        <Label htmlFor="inference-protocol">Wire protocol</Label>
        <Select
          value={inference?.wire_protocol ?? "none"}
          onValueChange={(value) =>
            form.setValue(
              "inference",
              value === "none"
                ? null
                : {
                    wire_protocol: value as InferenceMetadata["wire_protocol"],
                    model_list: inference?.model_list ?? false,
                    realtime: inference?.realtime ?? false,
                  },
              { shouldDirty: true, shouldValidate: true },
            )
          }
        >
          <SelectTrigger id="inference-protocol">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {[
              "none",
              "anthropic_messages",
              "openai_responses",
              "openai_completions",
            ].map((value) => (
              <SelectItem key={value} value={value}>
                {value === "none" ? "Not an inference service" : value}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {inference &&
          (["model_list", "realtime"] as const).map((field) => (
            <div
              key={field}
              className="flex items-center justify-between text-xs"
            >
              <Label htmlFor={`inference-${field}`}>
                {field === "model_list"
                  ? "Model list at /models"
                  : "Realtime WebSocket at /realtime"}
              </Label>
              <Switch
                id={`inference-${field}`}
                checked={inference[field] ?? false}
                onCheckedChange={(value) =>
                  form.setValue(
                    "inference",
                    { ...inference, [field]: value },
                    { shouldDirty: true },
                  )
                }
              />
            </div>
          ))}
      </section>
      <section className="space-y-3">
        <h3 className="text-[13px] font-semibold">Platform key</h3>
        <p className="text-xs text-muted-foreground">
          Offer a server-held credential to authorized users. Organization
          grants include members who can proxy services.
        </p>
        <FormField
          control={form.control}
          name="credential"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Replace platform credential</FormLabel>
              <FormControl>
                <Input
                  {...field}
                  value={field.value ?? ""}
                  type="password"
                  autoComplete="new-password"
                  placeholder="Leave blank to keep the current credential"
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <div className="flex items-center justify-between">
          <Label htmlFor="platform-key-enabled">Enable platform key</Label>
          <Switch
            id="platform-key-enabled"
            checked={platform.enabled}
            onCheckedChange={(enabled) => setPlatform({ enabled })}
          />
        </div>
        <Label htmlFor="platform-key-audience">Audience</Label>
        <Select
          value={platform.audience}
          onValueChange={(audience) =>
            setPlatform({ audience: audience as PlatformKeyConfig["audience"] })
          }
        >
          <SelectTrigger id="platform-key-audience">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="public">All authenticated users</SelectItem>
            <SelectItem value="restricted">
              Selected people and organizations
            </SelectItem>
          </SelectContent>
        </Select>
        {platform.audience === "restricted" && (
          <UserPicker
            selected={platform.allowed_owner_ids}
            onChange={(allowed_owner_ids) => setPlatform({ allowed_owner_ids })}
          />
        )}
      </section>
      <section className="space-y-3">
        <h3 className="text-[13px] font-semibold">Billing lanes</h3>
        {!form.watch("byok_pricing") && !form.watch("platform_key_pricing") && (
        <p className="text-xs text-muted-foreground">No lane prices are configured. Legacy billing below still applies.</p>
      )}
      <div className="grid gap-3 sm:grid-cols-2">
          {(["byok_pricing", "platform_key_pricing"] as const).map((field) => {
            const lane = form.watch(field);
            const status = service.billing?.[field]?.sync_status;
            const setLane = (value: LanePricingView | null) =>
              form.setValue(field, value, {
                shouldDirty: true,
                shouldValidate: true,
              });
            return (
              <div
                key={field}
                className="space-y-3 rounded-lg border border-border/50 p-3"
              >
                <div className="text-xs font-medium">
                  {field === "byok_pricing"
                    ? "Your own key"
                    : "NyxID platform key"}
                </div>
                <div className="flex items-center justify-between">
                  <Label htmlFor={`${field}-enabled`}>
                    {lane ? "Charge" : "Free"}
                  </Label>
                  <Switch
                    id={`${field}-enabled`}
                    checked={Boolean(lane)}
                    onCheckedChange={(enabled) =>
                      setLane(
                        enabled
                          ? { metric: "requests", credits_per_unit: "0" }
                          : null,
                      )
                    }
                  />
                </div>
                {lane && (
                  <>
                    <Label htmlFor={`${field}-metric`}>Unit</Label>
                    <Select
                      value={lane.metric}
                      onValueChange={(metric) =>
                        setLane({
                          ...lane,
                          metric: metric as LanePricingView["metric"],
                        })
                      }
                    >
                      <SelectTrigger id={`${field}-metric`}>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {["requests", "tokens", "bytes"].map((metric) => (
                          <SelectItem key={metric} value={metric}>
                            {metric}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FormField
                      control={form.control}
                      name={`${field}.credits_per_unit`}
                      render={({ field: input }) => (
                        <FormItem>
                          <FormLabel>Credits per unit</FormLabel>
                          <FormControl>
                            <Input {...input} inputMode="decimal" />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                    {status && (
                      <p className="text-[11px] text-muted-foreground">
                        Price sync: {status}
                      </p>
                    )}
                  </>
                )}
              </div>
            );
          })}
        </div>
        {(form.watch("byok_pricing") || form.watch("platform_key_pricing")) && (
          <p className="text-xs text-muted-foreground">
            Billing lanes supersede legacy platform billing below. A lane set to
            Free has no lane charge. Resale billing remains separate.
          </p>
        )}
      </section>
    </div>
  );
}
