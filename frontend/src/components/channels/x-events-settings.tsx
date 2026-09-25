import { useChannelPlatforms } from "@/hooks/use-channel-platforms";
import { useEffect } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useUpdateChannelBot } from "@/hooks/use-channel-bots";
import {
  xChannelEventsSchema,
  type XChannelEventsFormData,
} from "@/schemas/channels";
import type { ChannelBotDetail, XChannelEvent } from "@/types/channels";

export function XEventsSettings({ bot }: { readonly bot: ChannelBotDetail }) {
  const update = useUpdateChannelBot();
  const catalog = useChannelPlatforms();
  const choices =
    catalog.data?.platforms.find((entry) => entry.platform === bot.platform)
      ?.activities ?? [];
  const saved = (bot.x_events ?? ["dm"]).join(",");
  const form = useAppForm<XChannelEventsFormData>({
    resolver: zodResolver(xChannelEventsSchema),
    defaultValues: { events: [...(bot.x_events ?? ["dm"])] },
    mode: "onChange",
  });
  const { reset } = form;
  useEffect(() => {
    reset({ events: saved.split(",") as XChannelEvent[] });
  }, [bot.id, saved, reset]);
  const events = form.watch("events");

  function save(data: XChannelEventsFormData) {
    update.mutate(
      { id: bot.id, data: { x_events: data.events } },
      {
        onSuccess: () => {
          reset(data);
          toast.success("X event subscriptions updated");
        },
      },
    );
  }

  return (
    <form
      onSubmit={form.handleSubmit(save)}
      className="space-y-4 border-t border-border px-4 py-4"
    >
      <h3 className="text-sm font-medium">Events to receive</h3>
      {choices.map(
        ({ subscription, subscription_label, label, description }) => {
          const value = subscription as XChannelEvent;
          return (
            <div key={value} className="flex items-start gap-3">
              <Checkbox
                id={`${bot.id}-x-${value}`}
                checked={events.includes(value)}
                disabled={update.isPending || !bot.is_active}
                onCheckedChange={(checked) =>
                  form.setValue(
                    "events",
                    checked === true
                      ? [...events, value]
                      : events.filter((event) => event !== value),
                  )
                }
              />
              <div className="space-y-1">
                <Label htmlFor={`${bot.id}-x-${value}`}>
                  {subscription_label ?? label}
                </Label>
                <p className="text-xs text-muted-foreground">{description}</p>
              </div>
            </div>
          );
        },
      )}
      {catalog.error && (
        <ErrorBanner message="Event choices could not be loaded." />
      )}
      <p className="text-xs text-muted-foreground">
        Public replies support text. Reply events cover direct replies to your
        posts; protected posts are not delivered. Received events and sent
        replies use this service&apos;s request price. If write permission is
        missing, reconnect the account below and save again.
      </p>
      {form.formState.errors.events?.message && (
        <p role="alert" className="text-xs text-destructive">
          {form.formState.errors.events.message}
        </p>
      )}
      {update.error && <ErrorBanner message={update.error.message} />}
      <Button
        type="submit"
        isLoading={update.isPending}
        disabled={
          !bot.is_active ||
          !choices.length ||
          !form.formState.isDirty ||
          !form.formState.isValid ||
          update.isPending
        }
      >
        Save event subscriptions
      </Button>
    </form>
  );
}
