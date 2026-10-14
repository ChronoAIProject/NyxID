import { creditUnit } from "@/lib/platform-onboarding";
import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";
import {
  useGatewayCleanup,
  useGatewayCleanupAction,
  useAdminPlatformAction,
  useAdminGuideMutation,
  useAdminGuideServices,
  useAdminPlatformProfiles,
  useAdminPlatformTargets,
} from "@/hooks/use-platform-onboarding";
import {
  guideConfigurationSchema,
  type PlatformProfile,
  type GatewayCleanupReview,
} from "@/schemas/platform-onboarding";
import { Input } from "@/components/ui/input";
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
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/shared/page-header";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";

const convertSchema = z.object({
  channel_bot_id: z.uuid(),
  funding_owner_user_id: z.uuid(),
});
function GuideSettings({ profile }: { profile: PlatformProfile }) {
  const mutation = useAdminGuideMutation();
  const [after, setAfter] = useState<string>();
  const catalog = useAdminGuideServices(profile, after);
  const [models, setModels] = useState<string[]>([]);
  const form = useAppForm({
    resolver: zodResolver(guideConfigurationSchema),
    defaultValues: profile.guide ?? {
      model_service_id: "",
      model_id: "",
      limits: {
        daily_credits: "1",
        turns_per_hour: 10,
        concurrent_turns: 1,
        max_input_chars: 2048,
        max_output_tokens: 512,
      },
      anonymous_daily_credits: "0.1",
      platform_turns_per_hour: 100,
      platform_concurrent_turns: 4,
    },
  });
  return (
    <Form {...form}>
      <form
        className="space-y-3 rounded-lg border p-3"
        onSubmit={form.handleSubmit((guide) =>
          mutation.mutate({
            action: "configure",
            profile,
            guide,
            enabled: false,
          }),
        )}
      >
        <p>
          Guide inference uses the dedicated sponsor organization. Discovery and
          readiness checks may consume sponsor credits. No personal service
          connection is used.
        </p>
        {mutation.error && <ErrorBanner message={mutation.error.message} />}
        {catalog.error && (
          <ErrorBanner message="Could not load eligible sponsor-authorized platform inference services." />
        )}
        <FormField
          control={form.control}
          name="model_service_id"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Platform inference service</FormLabel>
              <FormControl>
                <select
                  {...field}
                  className="h-8 w-full rounded-lg border border-input bg-background px-3"
                  onChange={(event) => {
                    field.onChange(event.target.value);
                    form.setValue("model_id", "");
                    setModels([]);
                  }}
                >
                  <option value="">Choose an eligible catalog service</option>
                  {profile.guide &&
                    !catalog.data?.services.some(
                      (service) =>
                        service.id === profile.guide!.model_service_id,
                    ) && (
                      <option value={profile.guide.model_service_id}>
                        Current service (availability must be rechecked)
                      </option>
                    )}
                  {catalog.data?.services.map((service) => (
                    <option key={service.id} value={service.id}>
                      {service.name}
                    </option>
                  ))}
                </select>
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        {catalog.data?.next_after_id && (
          <Button
            type="button"
            onClick={() => setAfter(catalog.data!.next_after_id!)}
          >
            More eligible services
          </Button>
        )}
        {after && (
          <Button type="button" onClick={() => setAfter(undefined)}>
            First service page
          </Button>
        )}
        <FormField
          control={form.control}
          name="model_id"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Exact advertised model</FormLabel>
              <FormControl>
                <select
                  {...field}
                  className="h-8 w-full rounded-lg border border-input bg-background px-3"
                >
                  <option value="">Discover and select a model</option>
                  {field.value && !models.includes(field.value) && (
                    <option value={field.value}>
                      Current model: {field.value} (readiness required)
                    </option>
                  )}
                  {models.map((model) => (
                    <option key={model} value={model}>
                      {model}
                    </option>
                  ))}
                </select>
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />

        {(["anonymous_daily_credits"] as const).map((name) => (
          <FormField
            key={name}
            control={form.control}
            name={name}
            render={({ field }) => (
              <FormItem>
                <FormLabel>Anonymous sender daily credits</FormLabel>
                <FormControl>
                  <Input {...field} />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        ))}
        <FormField
          control={form.control}
          name="limits.daily_credits"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Bound sender daily credits</FormLabel>
              <FormControl>
                <Input {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        {(
          [
            ["limits.turns_per_hour", "Sender turns per hour"],
            ["limits.concurrent_turns", "Sender concurrent turns"],
            ["limits.max_input_chars", "Maximum input characters"],
            ["limits.max_output_tokens", "Maximum output tokens"],
            ["platform_turns_per_hour", "Platform turns per hour"],
            ["platform_concurrent_turns", "Platform concurrent turns"],
          ] as const
        ).map(([name, label]) => (
          <FormField
            key={name}
            control={form.control}
            name={name}
            render={({ field }) => (
              <FormItem>
                <FormLabel>{label}</FormLabel>
                <FormControl>
                  <Input
                    {...field}
                    type="number"
                    onChange={(event) =>
                      field.onChange(event.target.valueAsNumber)
                    }
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        ))}
        <p>
          Sender daily cap: {form.watch("limits.daily_credits")}{" "}
          {creditUnit(form.watch("limits.daily_credits"))}.{" "}
          {form.watch("limits.turns_per_hour")} turns/hour. Global limit:{" "}
          {form.watch("platform_turns_per_hour")} turns/hour,{" "}
          {form.watch("platform_concurrent_turns")} concurrent.
        </p>
        <div className="flex flex-wrap gap-2">
          <Button type="submit" disabled={mutation.isPending}>
            Save paused configuration
          </Button>
          <Button
            type="button"
            disabled={
              !z.uuid().safeParse(form.watch("model_service_id")).success ||
              mutation.isPending
            }
            onClick={() =>
              mutation.mutate(
                {
                  action: "discover",
                  profile,
                  model_service_id: form.getValues("model_service_id"),
                },
                {
                  onSuccess: (result) => {
                    if (
                      "models" in result &&
                      result.model_service_id ===
                        form.getValues("model_service_id") &&
                      result.generation === profile.generation
                    )
                      setModels(result.models);
                  },
                },
              )
            }
          >
            Discover sponsor-funded models
          </Button>
          <Button
            type="button"
            disabled={
              !profile.guide || form.formState.isDirty || mutation.isPending
            }
            onClick={() => {
              if (profile.guide)
                mutation.mutate({
                  action: "configure",
                  profile,
                  guide: profile.guide,
                  enabled: true,
                });
            }}
          >
            Validate model and enable guide
          </Button>
        </div>
        {models.length > 0 && <p>Advertised models: {models.join(", ")}</p>}
      </form>
    </Form>
  );
}

export function GatewayCleanupControls({
  profile,
}: {
  profile: PlatformProfile;
}) {
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
      <h3 className="text-12 font-medium">Previous gateway connections</h3>
      <p className="text-12 text-muted-foreground">
        Inspect a previous connection before resolving its cleanup. A changed
        remote binding is kept; recovery never deletes a different version.
      </p>
      {tasks.error && (
        <ErrorBanner message="Could not load previous gateway connections." />
      )}
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
      {tasks.data?.map((task) => (
        <div
          key={task.id}
          className="flex flex-wrap items-center justify-between gap-2 text-12"
        >
          <span>
            Previous connection · version{" "}
            {task.captured_version ?? "unavailable"}
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
          >
            Inspect connection
          </Button>
        </div>
      ))}
      {current && (
        <div className="space-y-3 text-12">
          <p>
            Reviewed previous version {current.captured_version}.{" "}
            {current.remote_exists
              ? `Current remote version ${current.observed_version}.`
              : "The remote connection is absent."}
          </p>
          <details>
            <summary>Connection identifiers</summary>
            <p className="break-all">{current.remote_channel_id}</p>
          </details>
          <p>
            {action === "preserve_changed"
              ? "Keep the changed remote binding and close cleanup for the obsolete connection. No remote connection will be deleted."
              : action === "confirm_missing"
                ? "Close cleanup for the absent remote connection."
                : "Retry deletion of the original reviewed version. A later version change will stop the retry."}
          </p>
          <div className="flex items-center gap-2">
            <Checkbox
              id={`gateway-review-${current.task_id}`}
              checked={confirmation === reviewKey}
              onCheckedChange={(checked) =>
                setConfirmation(checked ? reviewKey : undefined)
              }
            />
            <label htmlFor={`gateway-review-${current.task_id}`}>
              I reviewed this connection and resolution.
            </label>
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
              ? "Keep changed remote binding"
              : action === "confirm_missing"
                ? "Close completed cleanup"
                : "Retry original cleanup"}
          </Button>
        </div>
      )}
    </section>
  );
}

function AdminProfiles() {
  const [before, setBefore] = useState<string>();
  const profiles = useAdminPlatformProfiles(before);
  const action = useAdminPlatformAction();
  const targets = useAdminPlatformTargets();
  const [review, setReview] = useState<PlatformProfile>();
  const form = useAppForm({
    resolver: zodResolver(convertSchema),
    defaultValues: { channel_bot_id: "", funding_owner_user_id: "" },
  });
  return (
    <div className="space-y-6">
      <PageHeader
        title="Platform BotFather"
        description="Admin controls for the public onboarding manager. Client setups remain client-owned."
      />
      <DetailSection title="Convert a manager channel">
        <Form {...form}>
          <form
            className="space-y-4 p-4"
            onSubmit={form.handleSubmit((values) =>
              action.mutate(
                { action: "convert", ...values },
                { onSuccess: () => form.reset() },
              ),
            )}
          >
            <p className="text-12 text-muted-foreground">
              Conversion permanently closes personal routing, revokes old
              assistant and route keys, and starts paused. Select a dedicated
              organization you can administer as the funding destination.
            </p>
            {targets.error && (
              <ErrorBanner message="Could not load your manager channels and funding organizations." />
            )}
            {(["channel_bot_id", "funding_owner_user_id"] as const).map(
              (name) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={name}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {name === "channel_bot_id"
                          ? "Manager channel"
                          : "Dedicated funding organization"}
                      </FormLabel>
                      <FormControl>
                        <select
                          {...field}
                          className="h-8 w-full rounded-lg border border-input bg-background px-3 text-12"
                        >
                          <option value="">
                            Choose{" "}
                            {name === "channel_bot_id"
                              ? "a manager"
                              : "an organization"}
                          </option>
                          {name === "channel_bot_id"
                            ? targets.data?.bots.map((bot) => (
                                <option key={bot.id} value={bot.id}>
                                  {bot.label} · @{bot.platform_bot_username}
                                </option>
                              ))
                            : targets.data?.orgs.map((org) => (
                                <option key={org.id} value={org.id}>
                                  {org.display_name ?? org.slug}
                                </option>
                              ))}
                        </select>
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ),
            )}

            <div className="flex justify-end">
              <Button
                type="submit"
                variant="primary"
                disabled={
                  !form.formState.isDirty ||
                  !form.watch("channel_bot_id") ||
                  !form.watch("funding_owner_user_id")
                }
                isLoading={action.isPending}
              >
                Convert and pause manager
              </Button>
            </div>
          </form>
        </Form>
      </DetailSection>
      {action.error && <ErrorBanner message={action.error.message} />}
      {profiles.error && (
        <ErrorBanner
          message="Could not load platform profiles. Your live admin access may have changed."
          onRetry={() => void profiles.refetch()}
        />
      )}
      {profiles.isPending && (
        <p role="status" className="text-12">
          Loading platform profiles…
        </p>
      )}
      {profiles.data?.profiles.map((profile) => (
        <DetailSection
          key={profile.id}
          title={`Manager ${profile.manager_bot_id}`}
        >
          <div className="space-y-3 p-4 text-12">
            <Badge
              variant={
                profile.deleted_at
                  ? "secondary"
                  : profile.enabled
                    ? "success"
                    : "warning"
              }
            >
              {profile.deleted_at
                ? "Retired"
                : profile.enabled
                  ? "Enabled"
                  : "Paused"}
            </Badge>
            <p className="break-all">
              Profile: {profile.id} · Generation {profile.generation}
            </p>
            <p className="text-sm text-muted-foreground">
              Changing this profile pauses replies from existing client bots.
              Clients must review their setups again before replies resume.
            </p>
            <p className="break-all">
              Funding organization: {profile.funding_owner_user_id}
            </p>
            <p className="text-muted-foreground">
              The platform guide has no administrator inventory, private context
              or credentials. Repair retains the shared native creation webhook.
            </p>
            {!profile.deleted_at && (
              <GuideSettings
                key={`${profile.id}:${profile.generation}`}
                profile={profile}
              />
            )}
            <GatewayCleanupControls
              key={`cleanup:${profile.id}:${profile.generation}`}
              profile={profile}
            />
            {!profile.deleted_at && (
              <div className="flex flex-wrap gap-2">
                <Button
                  disabled={action.isPending}
                  onClick={() => action.mutate({ action: "verify", profile })}
                >
                  Verify manager
                </Button>
                <Button
                  disabled={action.isPending}
                  onClick={() => action.mutate({ action: "repair", profile })}
                >
                  Repair webhook
                </Button>
                <Button
                  disabled={action.isPending}
                  onClick={() => action.mutate({ action: "pause", profile })}
                >
                  Pause
                </Button>
                <Button
                  disabled={action.isPending}
                  onClick={() => setReview(profile)}
                >
                  Review retirement
                </Button>
              </div>
            )}
            {review?.id === profile.id && (
              <div className="space-y-2">
                <p>
                  Retire this profile and deactivate its channel? Retired
                  managers cannot return to personal routing.
                </p>
                {review.generation !== profile.generation && (
                  <>
                    <p>
                      This profile changed. Review its current configuration
                      before retiring.
                    </p>
                    <Button onClick={() => setReview(profile)}>
                      Review current profile
                    </Button>
                  </>
                )}
                <div className="flex justify-end gap-2">
                  <Button onClick={() => setReview(undefined)}>
                    Keep profile
                  </Button>
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
                  >
                    Retire platform manager
                  </Button>
                </div>
              </div>
            )}
          </div>
        </DetailSection>
      ))}
      {profiles.data?.profiles.length === 0 && (
        <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">
          No platform profiles on this page.
        </p>
      )}
      <div className="flex justify-end gap-2">
        {before && (
          <Button onClick={() => setBefore(undefined)}>Newest profiles</Button>
        )}
        {profiles.data?.next_before_id && (
          <Button
            onClick={() =>
              setBefore(profiles.data?.next_before_id ?? undefined)
            }
          >
            Older profiles
          </Button>
        )}
      </div>
      <Button asChild>
        <a href="/admin/platform-credentials">Manage platform credentials</a>
      </Button>
    </div>
  );
}
export function AdminPlatformBotsPage() {
  const user = useAuthStore((s) => s.user);
  return canAdminWrite(user) ? (
    <AdminProfiles key={user?.id} />
  ) : (
    <ErrorBanner message="Live NyxID admin access is required for platform BotFather controls." />
  );
}
