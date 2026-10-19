import { useState } from "react";
import { useAdminPlatformAction } from "@/hooks/use-platform-onboarding";
import { Input } from "@/components/ui/input";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useManagerRuntime,
  useManagerRuntimeAction,
} from "@/hooks/use-platform-runtime";
import {
  runtimePreconditions,
  type ManagerRuntime,
} from "@/schemas/platform-runtime";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  useAppForm,
} from "@/components/ui/form";

const modes = z.object({
  public_enabled: z.boolean(),
  personal_enabled: z.boolean(),
});
const statusLabels = {
  active: "Active",
  paused: "Paused",
  setup_incomplete: "Setup incomplete",
  connection_repair_required: "Connection repair required",
  retired: "Retired",
};
export function ManagerRuntimeSettings({
  profile,
}: {
  profile: PlatformProfile;
}) {
  const query = useManagerRuntime(profile.id);
  const mutation = useManagerRuntimeAction(profile.id);
  const [connectionBusy, setConnectionBusy] = useState(false);
  if (query.isPending) return <p role="status">Loading bot status…</p>;
  if (query.error)
    return (
      <ErrorBanner
        message={query.error.message}
        onRetry={() => void query.refetch()}
      />
    );
  if (!query.data.supported)
    return (
      <p role="status">Shared bot access is unavailable on this server.</p>
    );
  const runtime = query.data.runtime;
  const currentProfile = {
    ...profile,
    generation: query.data.profile_generation ?? profile.generation,
  };
  return (
    <>
      <RuntimeForm
        key={`${currentProfile.generation}:${runtime?.manager_authority_epoch}:${runtime?.personal_control_revision}:${runtime?.public_policy_revision}`}
        profile={currentProfile}
        value={query.data}
        mutation={mutation}
        connectionBusy={connectionBusy}
      />
      <ConnectionControls
        profile={currentProfile}
        value={query.data}
        disabled={mutation.isPending}
        onBusy={setConnectionBusy}
        onRefresh={() => void query.refetch()}
      />
    </>
  );
}
function RuntimeForm({
  profile,
  value,
  mutation,
  connectionBusy,
}: {
  profile: PlatformProfile;
  value: ManagerRuntime;
  mutation: ReturnType<typeof useManagerRuntimeAction>;
  connectionBusy: boolean;
}) {
  const busy = mutation.isPending || connectionBusy;
  const runtime = value.runtime;
  const stopped = runtime?.emergency_stopped ?? false;
  const repair = value.state === "connection_repair_required";
  const form = useAppForm<z.infer<typeof modes>>({
    resolver: zodResolver(modes),
    defaultValues: {
      public_enabled: stopped
        ? (runtime?.resume_public_enabled ?? true)
        : !runtime
          ? true
          : runtime.public_enabled,
      personal_enabled: stopped
        ? (runtime?.resume_personal_enabled ?? true)
        : !runtime
          ? true
          : runtime.personal_enabled,
    },
  });
  const expected = runtimePreconditions(profile.generation, runtime);
  return (
    <>
      <Form {...form}>
        <form
          className="space-y-4 rounded-xl border border-border bg-card p-4 text-12"
          onSubmit={form.handleSubmit((selected) =>
            mutation.mutate({
              action: "update",
              body: {
                ...expected,
                ...selected,
                emergency_stopped: false,
                public_policy: null,
              },
            }),
          )}
        >
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={value.state === "active" ? "success" : "secondary"}>
              {statusLabels[value.state]}
            </Badge>
            <span>
              {value.display_name} · {value.platform}
            </span>
          </div>
          <p>
            Choose approved public FAQ answers, personal NyxBot linking, or
            both.
          </p>
          <details>
            <summary className="cursor-pointer text-muted-foreground">
              How shared access works
            </summary>
            <p className="mt-2">
              Public FAQ uses approved content and requires no model, inference
              budget or sponsor. Personal linking requires NyxID sign-in,
              consent, and confirmation in the same private conversation. Each
              person uses their existing NyxBot, memory, permissions and
              billing. Provider transport charges stay with the connected bot's
              owner.
            </p>
            {value.platform === "x" && (
              <p>
                Paid X operations require configured channel billing and a
                provider plan that supports the selected webhook or polling
                mode.
              </p>
            )}
          </details>
          {value.reason === "legacy_public_policy_unavailable" && (
            <p role="status">
              The previous public AI configuration cannot serve replies. Enable
              the approved FAQ below to recover this bot.
            </p>
          )}
          {value.reason === "discord_commands_required" && (
            <p role="status">
              Configure Discord commands in Connection setup below, then enable
              the selected modes.
            </p>
          )}
          {repair && (
            <ErrorBanner message="The connection needs attention. Verify or repair it below, then resume the selected modes. An ownership change requires restoring the original owner or retiring this profile." />
          )}
          {(
            [
              ["public_enabled", "Public FAQ"],
              ["personal_enabled", "Personal NyxBot linking"],
            ] as const
          ).map(([name, label]) => (
            <FormField
              key={name}
              control={form.control}
              name={name}
              render={({ field }) => (
                <FormItem className="flex items-center gap-2">
                  <FormControl>
                    <Checkbox
                      disabled={busy}
                      checked={field.value}
                      onCheckedChange={(checked) =>
                        field.onChange(checked === true)
                      }
                    />
                  </FormControl>
                  <FormLabel>{label}</FormLabel>
                </FormItem>
              )}
            />
          ))}
          {!form.watch("public_enabled") && !form.watch("personal_enabled") && (
            <p role="status">
              Select at least one mode to enable or resume this bot.
            </p>
          )}
          {mutation.error && <ErrorBanner message={mutation.error.message} />}
          <div className="flex flex-wrap gap-2">
            <Button
              type="submit"
              variant="primary"
              isLoading={mutation.isPending}
              disabled={
                busy ||
                repair ||
                value.reason === "discord_commands_required" ||
                value.state === "retired" ||
                (!form.watch("public_enabled") &&
                  !form.watch("personal_enabled"))
              }
            >
              {stopped
                ? "Resume bot"
                : value.state === "active"
                  ? "Save modes"
                  : "Enable bot"}
            </Button>
            {runtime && !stopped && (
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() =>
                  mutation.mutate({
                    action: "update",
                    body: { ...expected, emergency_stopped: true },
                  })
                }
              >
                Pause bot
              </Button>
            )}
          </div>
          {stopped && (
            <p role="status">
              Public FAQ and new personal work are paused. Safe status notices
              and disconnect acknowledgments can still be sent. Resume restores
              the selected modes in one operation. Previously started work and
              history remain accessible in NyxID.
            </p>
          )}
          <details>
            <summary className="cursor-pointer font-medium">
              Review public FAQ
            </summary>
            <pre className="mt-3 whitespace-pre-wrap break-words font-sans">
              {value.faq_preview}
            </pre>
          </details>
          <p>
            {value.platform === "slack"
              ? "In a Slack DM, send the exact plain text help or faq <topic>, then link. After web consent, send confirm P-… in the same DM. Send unlink to disconnect. Slash commands are not used by this integration."
              : value.platform === "discord"
                ? "In an app DM, use /faq topic, /link, /confirm code, /chat message, /help and /unlink. Register the application commands described in connection setup below before testing."
                : "Test in a private conversation: send /faq, then /link. Sign in, review your account, and send the website's /confirm command in that exact conversation. Send /unlink to disconnect."}
          </p>
        </form>
      </Form>
    </>
  );
}

function ConnectionControls({
  profile,
  value,
  disabled,
  onBusy,
  onRefresh,
}: {
  profile: PlatformProfile;
  value: ManagerRuntime;
  disabled: boolean;
  onBusy: (busy: boolean) => void;
  onRefresh: () => void;
}) {
  const action = useAdminPlatformAction();
  const [notice, setNotice] = useState<string>();
  const [failure, setFailure] = useState<string>();
  const form = useAppForm({
    defaultValues: { fields: {} as Record<string, string>, connection_id: "" },
  });
  const run = (repair: boolean) =>
    form.handleSubmit((selected) => {
      if (disabled || action.isPending) return;
      onBusy(true);
      setNotice(undefined);
      setFailure(undefined);
      action.mutate(
        {
          action: repair ? "repair" : "verify",
          profile,
          ...(repair
            ? {
                fields: Object.fromEntries(
                  Object.entries(selected.fields).filter(([, v]) => v.length),
                ),
                connection_id: selected.connection_id || undefined,
              }
            : {}),
        },
        {
          onSuccess: () =>
            setNotice(
              repair
                ? "Connection repair saved. Complete provider setup and send a private test message (or use the provider dashboard test). Refresh status, then resume. Everyone must link again after repair."
                : "Provider identity verified. Complete provider setup and send a private test message to confirm webhook delivery.",
            ),
          onError: (error) => setFailure(error.message),
          onSettled: () => {
            form.reset();
            action.reset();
            onBusy(false);
          },
        },
      );
    })();
  return (
    <details className="rounded-lg border border-border p-3">
      <summary className="cursor-pointer font-medium">
        Connection setup and recovery
      </summary>
      <form
        className="mt-3 space-y-3"
        onSubmit={(event) => {
          event.preventDefault();
          void run(true);
        }}
      >
        {value.connection ? (
          <>
            <p>
              Connection status: {value.connection.status.replaceAll("_", " ")}.
              Callback URL:
            </p>
            <code className="block break-all select-all">
              {value.connection.webhook_url}
            </code>
            {value.connection.instructions.map((instruction, index) => (
              <p key={index}>{instruction}</p>
            ))}
            {value.platform === "discord" && (
              <p>
                Install the app with bot and applications.commands scopes, then
                open a DM with the bot. Configure commands here to add /link,
                /confirm code, /chat message, /faq topic, /help, /start and
                /unlink for bot DMs. Other application commands are preserved.
                Ordinary gateway messages do not reach this webhook.
              </p>
            )}
            {value.platform === "slack" && (
              <p>
                Subscribe to message.im with im:history and chat:write scopes.
                Enable App Home messages, reinstall after scope changes, and
                send the plain text help in a DM. Do not configure slash
                commands for these controls.
              </p>
            )}
            {value.connection.fields.map((field) => (
              <div key={field.name} className="space-y-1">
                <label htmlFor={`${profile.id}-${field.name}`}>
                  {field.label} replacement (optional)
                </label>
                <Input
                  id={`${profile.id}-${field.name}`}
                  type={field.secret ? "password" : "text"}
                  autoComplete="off"
                  {...form.register(`fields.${field.name}`)}
                />
              </div>
            ))}
            {value.connection.credential_source === "connection" && (
              <div>
                <p>
                  The bot owner must reconnect this same provider account in
                  NyxID first. An administrator can then adopt its owner-scoped
                  connection UUID below.
                </p>
                <label htmlFor={`${profile.id}-connection`}>
                  Replacement connection UUID (optional)
                </label>
                <Input
                  id={`${profile.id}-connection`}
                  {...form.register("connection_id")}
                />
              </div>
            )}
            {value.connection.credential_source === "platform" && (
              <a href="/admin/platform-credentials" className="underline">
                Update managed platform credentials
              </a>
            )}
          </>
        ) : (
          <p>
            Native Telegram manager credentials are managed in{" "}
            <a href="/admin/platform-credentials" className="underline">
              Admin platform credentials
            </a>
            . Verify checks this manager's webhook; Repair configures its
            dedicated manager callback.
          </p>
        )}
        <p>
          Repair pauses this bot, invalidates old links and pending replies, and
          keeps the same provider account. Resume the selected modes after
          connection setup succeeds.
        </p>
        {failure && <ErrorBanner message={failure} />}{" "}
        {notice && <p role="status">{notice}</p>}
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={disabled || action.isPending}
            onClick={() => void run(false)}
          >
            Verify connection
          </Button>
          <Button
            type="submit"
            variant="outline"
            disabled={disabled || action.isPending}
          >
            Repair connection
          </Button>
          {value.platform === "discord" && (
            <Button
              type="button"
              disabled={disabled || action.isPending}
              onClick={() => {
                onBusy(true);
                setFailure(undefined);
                setNotice(undefined);
                action.mutate(
                  { action: "discord-commands", profile },
                  {
                    onSettled: () => onBusy(false),
                    onSuccess: () =>
                      setNotice(
                        "Discord commands configured. Open a DM with the bot and use /help.",
                      ),
                    onError: (error) => setFailure(error.message),
                  },
                );
              }}
            >
              Configure Discord commands
            </Button>
          )}
          <Button
            type="button"
            variant="outline"
            disabled={disabled || action.isPending}
            onClick={onRefresh}
          >
            Refresh status
          </Button>
        </div>
      </form>
    </details>
  );
}
