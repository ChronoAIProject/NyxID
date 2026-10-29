import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useSaveWhatsappNative, useWhatsappNative, type WhatsappNativeSettings } from "@/hooks/use-whatsapp-native";
import { whatsappNativeSchema, type WhatsappNativeForm } from "@/schemas/whatsapp-native";
import { WhatsappRollbackTokenSection } from "./whatsapp-rollback-token";

export function WhatsappNativeSection({ botId }: { readonly botId: string }) {
  const query = useWhatsappNative(botId);
  // Older replicas have no endpoint; viewers cannot edit this owner-only panel.
  if (query.isError && query.error instanceof ApiError && [403, 404].includes(query.error.status)) return null;
  if (query.isError) return <ErrorBanner message="Could not load WhatsApp gateway settings." onRetry={() => void query.refetch()} />;
  if (!query.data || (!query.data.enabled && query.data.callback_phase === "relay")) return null;
  return <DetailSection title="Native WhatsApp gateway">
    {query.data.reason ? <p className="p-5 text-12 text-muted-foreground">{query.data.reason === "dedicated_byo_app_required"
      ? "Native WhatsApp requires your own dedicated Meta app. Managed Embedded Signup bots use a shared app and remain on the NyxID relay."
      : "An active WhatsApp bot with a Phone Number ID and Meta App Secret is required. Check the bot’s credentials before preparing native gateway access."}</p>
      : <>
        {query.data.callback_error === "relay_subscription_write_failed" && <p className="p-5 text-12 text-destructive">NyxID could not point Meta back to the relay automatically. Generate a replacement Verify Token below, then point this dedicated Meta app’s WhatsApp webhook to the relay URL with the new token.</p>}
        <NativeForm key={`${botId}:${query.data.revision}`} botId={botId} settings={query.data} />
        {query.data.callback_phase === "awaiting_relay" && <WhatsappRollbackTokenSection key={botId} botId={botId} revision={query.data.revision} />}
      </>}
  </DetailSection>;
}

function NativeForm({ botId, settings }: { readonly botId: string; readonly settings: WhatsappNativeSettings }) {
  const save = useSaveWhatsappNative(botId);
  const { register, handleSubmit, watch, setValue, formState: { errors, isDirty, isValid } } = useAppForm<WhatsappNativeForm>({
    resolver: zodResolver(whatsappNativeSchema), mode: "onChange",
    defaultValues: { meta_app_id: settings.meta_app_id ?? "", waba_id: settings.waba_id ?? "", dedicated_app: settings.dedicated_app },
  });
  const editable = settings.enabled && settings.callback_phase === "relay";
  return <form className="space-y-4 p-5" onSubmit={handleSubmit((data) => save.mutate({ ...data, revision: settings.revision }, {
    onSuccess: () => toast.success("Native WhatsApp identity saved"),
  }))}>
    <p className="text-12 text-muted-foreground">Use the Meta App ID from App settings → Basic, and the WhatsApp Business Account ID separately. Phone Number ID: {settings.phone_number_id}.</p>
    {([ ["meta_app_id", "Meta App ID"], ["waba_id", "WhatsApp Business Account ID (WABA)"] ] as const).map(([field, label]) =>
      <div className="space-y-2" key={field}>
        <Label htmlFor={`native-${field}`}>{label}</Label>
        <Input id={`native-${field}`} inputMode="numeric" autoComplete="off" maxLength={32} disabled={!editable || save.isPending} {...register(field)} />
        {errors[field] && <p role="alert" className="text-12 text-destructive">{errors[field].message}</p>}
      </div>)}
    <div className="flex items-start gap-3">
      <Switch id="native-dedicated" disabled={!editable || save.isPending} checked={watch("dedicated_app")} onCheckedChange={(value) => setValue("dedicated_app", value)} />
      <Label htmlFor="native-dedicated">This Meta app is dedicated to this bot and is not shared with other bots or Embedded Signup.</Label>
    </div>
    {errors.dedicated_app && <p role="alert" className="text-12 text-destructive">{errors.dedicated_app.message}</p>}
    <p className="text-12 text-muted-foreground">Saving prepares the identity only. When the flag is enabled, NyxID points this dedicated Meta app’s WhatsApp webhook to the CMA gateway and verifies the phone’s effective callback before swapping intake. Turning the flag off points the app back to the NyxID relay and verifies it before release; if Graph configuration fails, generate the one-time replacement Verify Token below and point Meta to the relay URL with it. Native groups, media and templates are unavailable; free-form replies keep WhatsApp’s 24-hour window.</p>
    {save.isError && <ErrorBanner message={save.error.message} />}
    <div className="flex justify-end"><Button variant="primary" type="submit" disabled={!editable || !isDirty || !isValid || save.isPending} isLoading={save.isPending}>Save identity</Button></div>
  </form>;
}
