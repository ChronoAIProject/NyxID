import { useState } from "react";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  Form,
  FormControl,
  FormField,
  FormLabel,
  useAppForm,
  FormDescription as RowDescription,
} from "@/components/ui/form";
import { GroupedField as FormItem } from "@/components/ui/grouped-fields";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import { GroupedList } from "@/components/ui/grouped-list";
import { Switch } from "@/components/ui/switch";
import { useNotificationsEnabled } from "@/hooks/use-assistant-notifications";
import { useNyxBotSettings, useUpdateNyxBotSettings } from "@/hooks/use-nyxbot-agents";
import { describeChanges } from "@/lib/grouped-form-changes";
import { NOTIFICATION_PUSH_ROWS } from "@/lib/assistant/notification-presentation";
import {
  DEFAULT_NOTIFICATION_PUSH,
  notificationPushFormSchema,
  type NotificationPushPreferences,
} from "@/schemas/assistant-notifications";

const onOff = (value: unknown) => (value ? translateUi("On") : translateUi("Off"));

function errorMessage(error: unknown): string {
  return error instanceof Error && error.message
    ? translateUi(error.message)
    : translateUi("Could not save notification preferences. Try again.");
}

/** NyxBot settings › Notifications: which kinds also reach the phone as push. */
export function NotificationPreferences() {
  useUiTranslation();
  const enabled = useNotificationsEnabled();
  const settings = useNyxBotSettings(enabled);
  if (!enabled || !settings.data) return null;
  return (
    <NotificationPreferencesForm
      key={JSON.stringify(settings.data.notifications?.push ?? DEFAULT_NOTIFICATION_PUSH)}
      defaults={settings.data.notifications?.push ?? DEFAULT_NOTIFICATION_PUSH}
    />
  );
}

function NotificationPreferencesForm({
  defaults,
}: {
  readonly defaults: NotificationPushPreferences;
}) {
  useUiTranslation();
  const update = useUpdateNyxBotSettings();
  const form = useAppForm<NotificationPushPreferences>({
    resolver: zodResolver(notificationPushFormSchema),
    defaultValues: defaults,
  });
  const [error, setError] = useState<string>();

  async function save(values: NotificationPushPreferences) {
    setError(undefined);
    try {
      const saved = await update.mutateAsync({ notifications: { push: values } });
      form.reset(saved.notifications?.push ?? values);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  return (
    <Form {...form}>
      <form
        aria-label={translateUi("Notification preferences")}
        noValidate
        onSubmit={form.handleSubmit(save)}
        className="space-y-6"
      >
        <GroupedList
          title={translateUi("Notifications")}
          footer={
            <>
              <p>
                {translateUi("Every kind appears in the Notifications bell. Push reaches the NyxID app on your phone; it is off until you turn a kind on here.")}
              </p>
              <p>
                {translateUi("Chat delivery of an automation's result is chosen per automation, not here.")}
              </p>
              {error ? (
                <p role="alert" className="text-destructive">
                  {error}
                </p>
              ) : null}
            </>
          }
        >
          {NOTIFICATION_PUSH_ROWS.map(({ name, label, description }) => (
            <FormField
              key={name}
              control={form.control}
              name={name}
              render={({ field }) => (
                <FormItem inline>
                  <div className="min-w-0 flex-1">
                    <FormLabel>{translateUi(label)}</FormLabel>
                    <RowDescription className="type-secondary text-text-tertiary">
                      {translateUi(description)}
                    </RowDescription>
                  </div>
                  <FormControl>
                    <Switch
                      aria-label={translateUi("Push for {{kind}}", { kind: translateUi(label).toLowerCase() })}
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                </FormItem>
              )}
            />
          ))}
        </GroupedList>
        <GroupedFormActions
          changes={() =>
            describeChanges(
              form.formState.defaultValues,
              form.getValues(),
              NOTIFICATION_PUSH_ROWS.map((row) => ({
                key: row.name,
                label: translateUi("Push: {{kind}}", { kind: translateUi(row.label) }),
                format: onOff,
              })),
            )
          }
          label={translateUi("Notification preferences")}
          dirty={form.formState.isDirty}
          pending={update.isPending}
          save={form.handleSubmit(save)}
          discard={() => {
            form.reset();
            setError(undefined);
          }}
          saveLabel={translateUi("Save notifications")}
        />
      </form>
    </Form>
  );
}
