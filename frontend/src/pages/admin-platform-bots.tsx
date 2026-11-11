import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { TelegramManagers } from "@/components/telegram-managers";
import {
  PlatformHelpEditor,
  PlatformHelpAssignment,
} from "@/components/platform-help-editor";
import { channelPlatformName } from "@/lib/assistant/nyxbot-labels";
import { NativePlatformEntrySetup } from "@/components/native-platform-entry-setup";
import { ManagerRuntimeSettings } from "@/components/platform-manager-runtime";
import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import {
  useGatewayCleanup,
  useGatewayCleanupAction,
  useAdminPlatformAction,
  useAdminPlatformProfiles,
  useAdminPlatformProfile,
  useAdminPlatformTargets,
} from "@/hooks/use-platform-onboarding";
import {
  type PlatformProfile,
  type GatewayCleanupReview,
} from "@/schemas/platform-onboarding";
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
import { Checkbox } from "@/components/ui/checkbox";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";

const convertSchema = z.object({
  channel_bot_id: z.uuid(),
});
export function GatewayCleanupControls({
  profile,
}: {
  profile: PlatformProfile;
}) {
  useUiTranslation();
  const actor = useAuthStore((s) => s.user?.id);
  const tasks = useGatewayCleanup(profile);
  const mutation = useGatewayCleanupAction(profile);
  const [review, setReview] = useState<{
    actor: string;
    value: GatewayCleanupReview;
  }>();
  const [confirmation, setConfirmation] = useState<string>();
  const current =
    review &&
    actor &&
    review.actor === actor &&
    review.value.profile_generation === profile.generation &&
    !review.value.completed &&
    tasks.data?.some(
      (task) =>
        task.id === review.value.task_id &&
        task.captured_version === review.value.captured_version,
    )
      ? review.value
      : undefined;
  const reviewKey = current
    ? `${actor}:${current.task_id}:${current.profile_generation}:${current.captured_version}:${current.observed_version}:${current.remote_exists}`
    : undefined;
  const action = current
    ? !current.remote_exists
      ? "confirm_missing"
      : current.observed_version === current.captured_version
        ? "retry_original"
        : "preserve_changed"
    : undefined;
  if (!tasks.data?.length && !tasks.error) return null;
  return (
    <section className="space-y-3 rounded-lg border border-border p-3">
      <h3 className="type-row-title">{translateUi("Previous gateway connections")}</h3>
      <p className="text-14 text-muted-foreground">{translateUi("Inspect a previous connection before resolving its cleanup. A changed remote binding is kept; recovery never deletes a different version.")}</p>
      {tasks.error && (
        <ErrorBanner message={translateUi("Could not load previous gateway connections.")} />
      )}
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
      {tasks.data?.map((task) => (
        <div
          key={task.id}
          className="flex flex-wrap items-center justify-between gap-2 text-14"
        >
          <span>{translateUi("Previous connection · version")}{" "}
            {task.captured_version ?? translateUi("unavailable")}
          </span>
          <Button
            disabled={!actor || mutation.isPending}
            onClick={() => {
              setReview(undefined);
              setConfirmation(undefined);
              mutation.mutate(
                { task: task.id, action: "inspect" },
                {
                  onSuccess: (value) => {
                    if (actor && value) setReview({ actor, value });
                  },
                },
              );
            }}
          >{translateUi("Inspect connection")}</Button>
        </div>
      ))}
      {current && (
        <div className="space-y-3 text-14">
          <p>{translateUi("Reviewed previous version ")}{current.captured_version}.{" "}
            {current.remote_exists
              ? translateUi("Current remote version {{value1}}.", { value1: current.observed_version })
              : translateUi("The remote connection is absent.")}
          </p>
          <details>
            <summary>{translateUi("Connection identifiers")}</summary>
            <p className="break-all">{current.remote_channel_id}</p>
          </details>
          <p>
            {action === "preserve_changed"
              ? translateUi("Keep the changed remote binding and close cleanup for the obsolete connection. No remote connection will be deleted.")
              : action === "confirm_missing"
                ? translateUi("Close cleanup for the absent remote connection.")
                : translateUi("Retry deletion of the original reviewed version. A later version change will stop the retry.")}
          </p>
          <div className="flex items-center gap-2">
            <Checkbox
              id={`gateway-review-${current.task_id}`}
              checked={confirmation === reviewKey}
              onCheckedChange={(checked) =>
                setConfirmation(checked ? reviewKey : undefined)
              }
            />
            <Label htmlFor={`gateway-review-${current.task_id}`}>{translateUi("I reviewed this connection and resolution.")}</Label>
          </div>
          <Button
            disabled={
              !actor ||
              !action ||
              !reviewKey ||
              confirmation !== reviewKey ||
              mutation.isPending
            }
            onClick={() => {
              if (!current || !action) return;
              mutation.mutate(
                { task: current.task_id, action, review: current },
                {
                  onSuccess: () => {
                    setReview(undefined);
                    setConfirmation(undefined);
                  },
                },
              );
            }}
          >
            {action === "preserve_changed"
              ? translateUi("Keep changed remote binding")
              : action === "confirm_missing"
                ? translateUi("Close completed cleanup")
                : translateUi("Retry original cleanup")}
          </Button>
        </div>
      )}
    </section>
  );
}

function AdminProfiles() {
  useUiTranslation();
  const [prepared, setPrepared] = useState<PlatformProfile>();
  const [notice, setNotice] = useState<string>();
  const pinned = useAdminPlatformProfile(prepared);
  const onPrepared = (profile: PlatformProfile) => {
    setPrepared(profile);
    setNotice(
      "Shared connection saved and paused. Review access below, then enable or resume. After repair everyone must link again.",
    );
  };
  const [before, setBefore] = useState<string>();
  const profiles = useAdminPlatformProfiles(before);
  const action = useAdminPlatformAction();
  const targets = useAdminPlatformTargets();
  const [review, setReview] = useState<PlatformProfile>();
  const form = useAppForm({
    resolver: zodResolver(convertSchema),
    defaultValues: { channel_bot_id: "" },
  });
  return (
    <div className="space-y-6">
      <PageHeader
        title={translateUi("Platform BotFather")}
        description={translateUi("Shared FAQ and personal NyxBot access. Review existing bots or prepare another channel.")}
      />
      <PlatformHelpEditor />
      <TelegramManagers />
      {action.error && <ErrorBanner message={action.error.message} />}
      {profiles.error && (
        <ErrorBanner
          message={translateUi("Could not load platform profiles. Your live admin access may have changed.")}
          onRetry={() => void profiles.refetch()}
        />
      )}
      {profiles.isPending && (
        <p role="status" className="text-14">{translateUi("Loading platform profiles…")}</p>
      )}
      {notice && <p role="status">{notice}</p>}
      {[
        ...(pinned.data ? [pinned.data] : []),
        ...(profiles.data?.profiles ?? []).filter(
          (profile) => profile.id !== pinned.data?.id,
        ),
      ].map((profile) => (
        <DetailSection
          key={profile.id}
          title={`${profile.display_name ?? profile.manager_bot_id} · ${channelPlatformName(profile.platform ?? "telegram")}`}
        >
          <div className="space-y-3 p-4 text-14">
            {profile.deleted_at && <p>{translateUi("Retired")}</p>}
            {!profile.deleted_at && (
              <>
                <ManagerRuntimeSettings profile={profile} />
                <PlatformHelpAssignment profile={profile} />
              </>
            )}
            {!profile.deleted_at && (
              <details>
                <summary>{translateUi("Repair connection")}</summary>
                {profile.execution_profile === "native_entry" ? (
                  <NativePlatformEntrySetup
                    profile={profile}
                    onPrepared={onPrepared}
                  />
                ) : (
                  <div className="flex flex-wrap gap-2">
                    <Button
                      disabled={action.isPending}
                      onClick={() =>
                        action.mutate({ action: "verify", profile })
                      }
                    >{translateUi("Verify manager")}</Button>
                    <Button
                      disabled={action.isPending}
                      onClick={() =>
                        action.mutate(
                          { action: "repair", profile },
                          { onSuccess: onPrepared },
                        )
                      }
                    >{translateUi("Repair webhook")}</Button>
                  </div>
                )}
              </details>
            )}
            <details>
              <summary>{translateUi("Identifiers, cleanup and retirement")}</summary>
              <p className="break-all">{translateUi("Profile: ")}{profile.id}{translateUi(" · Generation ")}{profile.generation}
              </p>
              <p>{translateUi("Setup owner: ")}{profile.funding_owner_user_id} ·{" "}
                {profile.transport ?? translateUi("direct")}{translateUi(" transport")}</p>
              <GatewayCleanupControls
                key={`cleanup:${profile.id}:${profile.generation}`}
                profile={profile}
              />
              {!profile.deleted_at && (
                <div className="flex flex-wrap gap-2">
                  <Button
                    disabled={action.isPending}
                    onClick={() => setReview(profile)}
                  >{translateUi("Review retirement")}</Button>
                </div>
              )}
              {review?.id === profile.id && (
                <div className="space-y-2">
                  <p>{translateUi("Retire this profile and deactivate its channel? Retired managers cannot return to personal routing.")}</p>
                  {review.generation !== profile.generation && (
                    <>
                      <p>{translateUi("This profile changed. Review its current configuration before retiring.")}</p>
                      <Button onClick={() => setReview(profile)}>{translateUi("Review current profile")}</Button>
                    </>
                  )}
                  <div className="flex justify-end gap-2">
                    <Button onClick={() => setReview(undefined)}>{translateUi("Keep profile")}</Button>
                    <Button
                      variant="destructive"
                      isLoading={action.isPending}
                      disabled={review.generation !== profile.generation}
                      onClick={() =>
                        action.mutate(
                          { action: "retire", profile: review },
                          { onSuccess: () => setReview(undefined) },
                        )
                      }
                    >{translateUi("Retire platform manager")}</Button>
                  </div>
                </div>
              )}
            </details>
          </div>
        </DetailSection>
      ))}
      {profiles.data?.profiles.length === 0 && (
        <p className="rounded-lg border border-dashed border-border p-4 text-14 text-muted-foreground">{translateUi("No platform profiles on this page.")}</p>
      )}
      <div className="flex justify-end gap-2">
        {before && (
          <Button onClick={() => setBefore(undefined)}>{translateUi("Newest profiles")}</Button>
        )}
        {profiles.data?.next_before_id && (
          <Button
            onClick={() =>
              setBefore(profiles.data?.next_before_id ?? undefined)
            }
          >{translateUi("Older profiles")}</Button>
        )}
      </div>
      <Button asChild>
        <a href="/admin/platform-credentials">{translateUi("Manage platform credentials")}</a>
      </Button>
      <details>
        <summary>{translateUi("Prepare another shared bot")}</summary>{" "}
        <DetailSection title={translateUi("Convert a manager channel")}>
          <Form {...form}>
            <form
              className="space-y-4 p-4"
              onSubmit={form.handleSubmit((values) =>
                action.mutate(
                  { action: "convert", ...values },
                  {
                    onSuccess: (profile) => {
                      form.reset();
                      onPrepared(profile);
                    },
                  },
                ),
              )}
            >
              <p className="text-14 text-muted-foreground">{translateUi("Conversion replaces the channel owner’s private route with shared access, revokes old assistant and route keys, and starts paused. Built-in FAQ needs no model or funding organization.")}</p>
              {targets.error && (
                <ErrorBanner message={translateUi("Could not load your manager channels.")} />
              )}
              <FormField
                control={form.control}
                name="channel_bot_id"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{translateUi("Telegram manager channel")}</FormLabel>
                    <FormControl>
                      <select
                        {...field}
                        className="h-8 w-full rounded-lg border border-input bg-background px-3 text-14"
                      >
                        <option value="">{translateUi("Choose a manager")}</option>
                        {targets.data?.bots
                          .filter(
                            (bot) =>
                              bot.credential_source === "telegram_manager",
                          )
                          .map((bot) => (
                            <option key={bot.id} value={bot.id}>
                              {bot.label}
                            </option>
                          ))}
                      </select>
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <div className="flex justify-end">
                <Button
                  type="submit"
                  variant="primary"
                  disabled={
                    !form.formState.isDirty || !form.watch("channel_bot_id")
                  }
                  isLoading={action.isPending}
                >{translateUi("Convert and pause manager")}</Button>
              </div>
            </form>
          </Form>
        </DetailSection>
        <NativePlatformEntrySetup onPrepared={onPrepared} />
      </details>
    </div>
  );
}
export function AdminPlatformBotsPage() {
  useUiTranslation();
  const user = useAuthStore((s) => s.user);
  return canAdminWrite(user) ? (
    <AdminProfiles key={user?.id} />
  ) : (
    <ErrorBanner message={translateUi("Live NyxID admin access is required for platform BotFather controls.")} />
  );
}
