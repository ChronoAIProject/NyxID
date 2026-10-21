import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useTelegramManagers,
  useConfigureTelegramManager,
  useClearTelegramManager,
} from "@/hooks/use-telegram-managers";
import { telegramManagerInputSchema } from "@/schemas/telegram-managers";
import {
  useAppForm,
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";

export function TelegramManagers() {
  const managers = useTelegramManagers();
  const save = useConfigureTelegramManager();
  const clear = useClearTelegramManager();
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [removing, setRemoving] = useState<string>();
  const form = useAppForm({
    resolver: zodResolver(telegramManagerInputSchema),
    defaultValues: { token: "" },
  });
  return (
    <DetailSection title="Telegram managers">
      <div className="space-y-4 p-4 text-12">
        <p>
          Connect additional BotFather managers here. Enable bot management in
          BotFather for each one. Each manager has its own webhook and can use
          any shared help library.
        </p>
        {managers.isPending && <p role="status">Loading Telegram managers…</p>}
        {managers.error && (
          <ErrorBanner
            message="Could not load Telegram managers."
            onRetry={() => void managers.refetch()}
          />
        )}
        {managers.data?.map((m) => (
          <div
            key={m.manager_bot_id}
            className="space-y-2 rounded-lg border border-border p-3"
          >
            <p>
              @{m.username} · {m.ready ? "Connected" : "Needs configuration"}
              {m.legacy_default
                ? " · Default (managed in Platform Credentials)"
                : ""}
            </p>
            {!m.legacy_default &&
              (removing === m.manager_bot_id ? (
                <div className="space-y-2">
                  <p>
                    Clear @{m.username}'s saved manager configuration? Delete
                    its channel connection and managed children, and finish or
                    cancel its creation requests first. This does not delete the
                    bot from Telegram.
                  </p>
                  <Button
                    variant="destructive"
                    isLoading={clear.isPending}
                    onClick={() =>
                      clear.mutate(m.manager_bot_id, {
                        onSuccess: () => setRemoving(undefined),
                      })
                    }
                  >
                    Clear @{m.username}
                  </Button>
                  <Button
                    disabled={clear.isPending}
                    onClick={() => {
                      clear.reset();
                      setRemoving(undefined);
                    }}
                  >
                    Keep manager
                  </Button>
                </div>
              ) : (
                <Button
                  onClick={() => {
                    clear.reset();
                    setRemoving(m.manager_bot_id);
                  }}
                >
                  Review clearing @{m.username}
                </Button>
              ))}
          </div>
        ))}
        {clear.error && <ErrorBanner message={clear.error.message} />}
        <Form {...form}>
          <form
            className="space-y-3"
            onSubmit={form.handleSubmit(({ token }) => {
              form.reset();
              setError("");
              setNotice("");
              save.mutate(token, {
                onSuccess: (m) =>
                  setNotice(
                    `@${m.username} is connected. Add it as a Telegram channel with its token, then convert that channel below and choose its help library.`,
                  ),
                onError: (failure) => setError(failure.message),
                onSettled: () => save.reset(),
              });
            })}
          >
            <FormField
              control={form.control}
              name="token"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Additional manager bot token</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      type="password"
                      autoComplete="off"
                      data-sensitive
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <Button type="submit" variant="primary" isLoading={save.isPending}>
              Connect manager
            </Button>
            {error && <ErrorBanner message={error} />}
          </form>
        </Form>
        {notice && <p role="status">{notice}</p>}
      </div>
    </DetailSection>
  );
}
