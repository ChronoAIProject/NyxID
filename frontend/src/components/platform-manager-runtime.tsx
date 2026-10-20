import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useManagerRuntime,
  useManagerRuntimeAction,
} from "@/hooks/use-platform-runtime";
import { useAdminPlatformTargets } from "@/hooks/use-platform-onboarding";
import {
  publicSetupSchema,
  runtimePreconditions,
  type ManagerRuntime,
  type PublicSetup,
  type PublicReadiness,
} from "@/schemas/platform-runtime";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ApiError } from "@/lib/api-client";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";

const reasons: Record<NonNullable<PublicReadiness["reason"]>, string> = {
  unsupported_input: "Use a text question or the available channel controls.",
  public_turn_failed:
    "The public turn failed. Try again to start a fresh turn.",
  confirmation_rejected:
    "Request a fresh account review and confirm in the same private chat.",
  control_invalid: "Use the bot’s help control to review supported commands.",
  control_rate_limited: "Wait a moment, then retry the chat control.",
  mode_disabled: "Public access is not available on this server.",
  manager_unavailable: "The manager is unavailable.",
  emergency_stopped:
    "The bot is paused. Select the modes you want, then choose Resume bot.",
  personal_disabled: "Personal access is paused.",
  public_disabled: "Public access is paused.",
  owner_unavailable: "The owner account is unavailable.",
  transport_unavailable:
    "Connection repair required. Use Repair connection below, then refresh status.",
  origin_revoked: "This authorization was revoked.",
  engine_unavailable: "NyxAgent is unavailable.",
  insufficient_funds: "The sponsor has insufficient credits.",
  capacity_limited: "Public capacity is currently full.",
  public_not_ready: "The restricted public runtime is not ready.",
  public_quota_limited:
    "The public budget cannot admit this profile’s bounded turn, or today’s quota is exhausted. Increase the configured cap or choose a lower-cost public profile.",
  runtime_uninitialized: "Choose the settings below to enable this bot.",
  sponsor_unavailable: "The sponsor organization is unavailable.",
  public_contract_unsupported:
    "The deployed NyxAgent does not support public bots yet. Upgrade it to a version with the public runtime contract, then refresh profiles and validate again.",
  public_authority_rejected:
    "NyxAgent rejected the public authority assertion. Verify that its version supports the public contract and that it trusts this NyxID issuer, JWKS and audience, then validate again.",
  profile_unavailable: "The selected public profile is unavailable.",
  proof_expired:
    "Public readiness needs a fresh check. Enabled bots refresh it automatically on the next public message; validate again to check it now.",
};
function runtimeFailure(error: unknown, fallback: string): string {
  if (!(error instanceof ApiError)) return fallback;
  if (error.errorCode === 11300)
    return "The sponsor has insufficient credits for the readiness check. Add credits to the sponsor organization and validate again.";
  if (error.errorCode === 11302)
    return "Billing could not authorize the readiness check. Check sponsor billing and the public model’s synchronized prices, then validate again.";
  if (error.status === 429)
    return "Public validation is temporarily limited. Wait a moment, check the remaining public budget, and validate again.";
  return fallback;
}
export function ManagerRuntimeSettings({
  profile,
}: {
  profile: PlatformProfile;
}) {
  const query = useManagerRuntime(profile.id);
  if (query.isPending)
    return (
      <p role="status" className="text-12">
        Loading bot access settings...
      </p>
    );
  if (query.error)
    return (
      <ErrorBanner
        message="Public and personal bot access is unavailable. The server may not support this feature yet."
        onRetry={() => void query.refetch()}
      />
    );
  if (!query.data.supported)
    return (
      <p role="status" className="text-12 text-muted-foreground">
        Public and personal bot access is not supported by this server.
      </p>
    );
  const runtime = query.data.runtime;
  return (
    <div className="space-y-3">
      <BasicRuntimeForm
        key={`${profile.generation}:${runtime?.manager_authority_epoch ?? 0}:${runtime?.personal_control_revision ?? 0}:${runtime?.public_policy_revision ?? 0}`}
        profile={profile}
        value={query.data}
      />
      <Button
        variant="outline"
        onClick={() => void query.refetch()}
        disabled={query.isFetching}
      >
        Refresh status
      </Button>
    </div>
  );
}
function BasicRuntimeForm({
  profile,
  value,
}: {
  profile: PlatformProfile;
  value: ManagerRuntime;
}) {
  const runtime = value.runtime;
  const stopped = runtime?.emergency_stopped === true;
  const mutation = useManagerRuntimeAction(profile.id);
  const [personal, setPersonal] = useState(
    stopped
      ? (runtime?.remembered_personal_enabled ?? false)
      : (runtime?.personal_enabled ?? true),
  );
  const [publicEnabled, setPublic] = useState(
    stopped
      ? (runtime?.remembered_public_enabled ?? false)
      : (runtime?.public_enabled ?? true),
  );
  const [source, setSource] = useState(runtime?.public_policy ? "ai" : "faq");
  const [failure, setFailure] = useState<string>();
  const [aiRecovery, setAiRecovery] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  async function save(pause = false) {
    setFailure(undefined);
    setAiRecovery(false);
    try {
      await mutation.mutateAsync({
        action: "update",
        body: {
          ...runtimePreconditions(profile.generation, runtime),
          ...(pause
            ? { emergency_stopped: true }
            : {
                ...(!runtime ||
                stopped ||
                initializedOff ||
                personal !== runtime.personal_enabled
                  ? { personal_enabled: personal }
                  : {}),
                ...(!runtime ||
                stopped ||
                initializedOff ||
                publicEnabled !== runtime.public_enabled
                  ? { public_enabled: publicEnabled }
                  : {}),
                ...(stopped ? { emergency_stopped: false } : {}),
                ...(source === "faq" && runtime?.public_policy
                  ? { public_policy: null }
                  : {}),
              }),
        },
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : "";
      setAiRecovery(
        !pause &&
          publicEnabled &&
          source === "ai" &&
          !!runtime?.public_policy &&
          (/public|nyxagent|sponsor|model|readiness|deployment|billing|credits/i.test(
            message,
          ) ||
            (error instanceof ApiError &&
              (error.status === 429 ||
                [11300, 11302].includes(error.errorCode ?? 0)))),
      );
      setFailure(
        error instanceof Error
          ? error.message
          : "Could not save access. Refresh status and try again.",
      );
    }
  }
  const state =
    value.state ??
    (stopped
      ? "paused"
      : runtime?.personal_enabled || runtime?.public_enabled
        ? "active"
        : "setup_incomplete");
  const active = state === "active";
  const initializedOff =
    !!runtime && !runtime.personal_enabled && !runtime.public_enabled;
  const statusLabel = {
    active: "Bot active",
    paused: "Bot paused",
    setup_incomplete: "Setup required",
    connection_repair_required: "Repair required",
    retired: "Bot retired",
  }[state];
  const statusText = {
    active: runtime?.personal_enabled
      ? runtime.public_enabled
        ? "Public answers are enabled. Personal NyxBot is active."
        : "Personal NyxBot is active. Public answers are off."
      : "Public answers are enabled. Personal NyxBot is off.",
    paused:
      "The bot is paused. Select the modes you want, then choose Resume bot.",
    setup_incomplete: "Select the modes you want, then choose Enable bot.",
    connection_repair_required:
      "Connection repair required. Use Repair connection below, then refresh status. Your saved mode choices are retained.",
    retired:
      "This bot is retired. Prepare another shared bot to enable access.",
  }[state];
  return (
    <section className="space-y-4 rounded-xl border border-border bg-card p-4 text-12">
      <div className="flex flex-wrap gap-2">
        <Badge variant={active ? "success" : "warning"}>{statusLabel}</Badge>
        <Badge
          variant={active && runtime?.public_enabled ? "success" : "secondary"}
        >
          Public{" "}
          {runtime?.public_enabled
            ? active
              ? "enabled"
              : "configured"
            : "off"}
        </Badge>
        <Badge
          variant={
            active && runtime?.personal_enabled ? "success" : "secondary"
          }
        >
          Personal{" "}
          {runtime?.personal_enabled
            ? active
              ? "enabled"
              : "configured"
            : "off"}
        </Badge>
      </div>
      <p role="status">{statusText}</p>
      {active &&
        runtime?.public_enabled &&
        !runtime.public_policy &&
        value.public_readiness.status === "faq_ready" && (
          <p>Built-in FAQ is ready. No model or sponsor is required.</p>
        )}
      {active && runtime?.public_enabled && runtime.public_policy && (
        <p>
          {value.public_readiness.status === "ready"
            ? "The configured public AI deployment is ready."
            : reasons[value.public_readiness.reason ?? "public_not_ready"]}
        </p>
      )}
      <form
        className="space-y-3"
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <label className="flex items-start gap-2">
          <Checkbox
            checked={publicEnabled}
            onCheckedChange={(value) => setPublic(value === true)}
          />
          <span>
            Public FAQ — approved help for visitors, without personal data.
          </span>
        </label>
        <label className="flex items-start gap-2">
          <Checkbox
            checked={personal}
            onCheckedChange={(value) => setPersonal(value === true)}
          />
          <span>
            Personal NyxBot — sign in, review consent and confirm in this
            private conversation.
          </span>
        </label>
        {runtime?.public_policy ? (
          <label className="block space-y-1">
            Public answers
            <select
              aria-label="Public answers"
              value={source}
              onChange={(event) => setSource(event.target.value)}
              className="h-8 w-full rounded-lg border border-input bg-background px-3"
            >
              <option value="faq">
                Built-in approved FAQ (no inference credits)
              </option>
              <option value="ai">Saved public AI configuration</option>
            </select>
          </label>
        ) : (
          <p className="text-muted-foreground">
            Public answers use the built-in approved FAQ. No inference credits
            or model setup are required.
          </p>
        )}
        {!personal && !publicEnabled && (
          <p>Select at least one mode to enable or resume this bot.</p>
        )}
        {failure && <ErrorBanner message={failure} />}
        {aiRecovery && (
          <p>
            To continue without public AI, choose Built-in approved FAQ or turn
            off Public FAQ, then save or resume. Personal NyxBot can run without
            a public AI deployment.
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button
            type="submit"
            variant="primary"
            disabled={
              mutation.isPending ||
              state === "retired" ||
              (!personal && !publicEnabled)
            }
          >
            {stopped
              ? "Resume bot"
              : !runtime || initializedOff
                ? "Enable bot"
                : "Save modes"}
          </Button>
          {runtime && !stopped && (
            <Button
              type="button"
              disabled={mutation.isPending}
              onClick={() => void save(true)}
            >
              Pause bot
            </Button>
          )}
        </div>
      </form>
      <p className="text-muted-foreground">
        Pause stops FAQ and personal work. Personal turns use each person's
        existing NyxBot permissions, memory and normal account billing in a
        separate saved channel thread.
      </p>
      <details onToggle={(event) => setAdvanced(event.currentTarget.open)}>
        <summary className="cursor-pointer">
          Advanced public AI deployment
        </summary>
        {advanced && <AiRuntimeForm profile={profile} value={value} />}
      </details>
    </section>
  );
}

export function AiRuntimeForm({
  profile,
  value,
}: {
  profile: PlatformProfile;
  value: ManagerRuntime;
}) {
  const runtime = value.runtime;
  const policy = runtime?.public_policy;
  const mutation = useManagerRuntimeAction(profile.id);
  const targets = useAdminPlatformTargets(true);
  const [profiles, setProfiles] = useState<{ id: string; label: string }[]>([]);
  const [readiness, setReadiness] = useState(value.public_readiness);
  const [error, setError] = useState<string>();
  const form = useAppForm<PublicSetup>({
    resolver: zodResolver(publicSetupSchema),
    defaultValues: {
      source: "nyxagent",
      profile: policy?.profile ?? "",
      total_daily_credits:
        policy?.total_daily_credits ??
        profile.guide?.limits.daily_credits ??
        "10",
      sender_daily_credits:
        policy?.sender_daily_credits ??
        profile.guide?.anonymous_daily_credits ??
        "0.5",
      sponsor_owner_user_id:
        policy?.sponsor_owner_user_id ??
        (targets.data?.orgs.some(
          (org) => org.id === profile.funding_owner_user_id,
        )
          ? profile.funding_owner_user_id
          : ""),
    },
  });
  const expected = runtimePreconditions(profile.generation, runtime);
  const pending = mutation.isPending;
  const stopped = runtime?.emergency_stopped === true || !!profile.deleted_at;
  const selection = form.watch("profile");
  function publicBody(values: PublicSetup) {
    const { sponsor_owner_user_id, ...rest } = values;
    return {
      ...expected,
      public_policy: {
        ...rest,
        ...(rest.profile ? {} : { profile: undefined }),
        ...(sponsor_owner_user_id !== policy?.sponsor_owner_user_id
          ? { sponsor_owner_user_id }
          : {}),
      },
    };
  }
  async function discover() {
    const parsed = publicSetupSchema.safeParse(form.getValues());
    if (!parsed.success) return;
    setError(undefined);
    try {
      const result = await mutation.mutateAsync({
        action: "profiles",
        body: publicBody(parsed.data),
      });
      if ("profiles" in result) {
        setProfiles(result.profiles);
        setReadiness(result.readiness);
        if (
          !policy &&
          !form.getFieldState("profile").isDirty &&
          result.default_profile
        )
          form.setValue("profile", result.default_profile, {
            shouldDirty: false,
            shouldTouch: false,
          });
      }
    } catch (error) {
      setError(
        runtimeFailure(
          error,
          "Could not discover public NyxAgent profiles. Check NyxAgent connectivity and public contract compatibility, then refresh profiles.",
        ),
      );
    }
  }
  async function enable(values: PublicSetup) {
    setError(undefined);
    try {
      let body = publicBody(values);
      const discovery = await mutation.mutateAsync({
        action: "profiles",
        body,
      });
      if (!("profiles" in discovery)) throw new Error();
      setProfiles(discovery.profiles);
      setReadiness(discovery.readiness);
      if (discovery.profiles.length === 0) {
        setError(reasons[discovery.readiness.reason ?? "public_not_ready"]);
        return;
      }
      if (
        !policy &&
        !form.getFieldState("profile").isDirty &&
        discovery.default_profile
      ) {
        values = { ...values, profile: discovery.default_profile };
        form.setValue("profile", values.profile, {
          shouldDirty: false,
          shouldTouch: false,
        });
        body = publicBody(values);
      }
      if (!discovery.profiles.some((row) => row.id === values.profile)) {
        setError(
          values.profile
            ? reasons.profile_unavailable
            : "Choose an advertised public profile below. This runtime has no public default.",
        );
        return;
      }
      const proof = await mutation.mutateAsync({ action: "readiness", body });
      if (!("public_readiness" in proof)) throw new Error();
      setReadiness(proof.public_readiness);
      if (
        proof.public_readiness.status !== "ready" ||
        !proof.public_readiness.expires_at ||
        Date.parse(proof.public_readiness.expires_at) <= Date.now()
      ) {
        setError(
          `Public access is unavailable. Settings were not enabled. ${reasons[proof.public_readiness.reason ?? "public_not_ready"]}`,
        );
        return;
      }
      await mutation.mutateAsync({
        action: "update",
        body: {
          ...body,
          ...(!runtime ? { personal_enabled: true } : {}),
          public_enabled: true,
          ...(proof.public_readiness.policy_digest
            ? { public_deployment_digest: proof.public_readiness.policy_digest }
            : {}),
        },
      });
    } catch (error) {
      setError(
        runtimeFailure(
          error,
          "Could not enable public access. Reload the current settings and check NyxAgent readiness and callback connectivity before retrying.",
        ),
      );
    }
  }
  return (
    <Form {...form}>
      <form
        className="space-y-4 rounded-xl border border-border bg-card p-4 text-12"
        onSubmit={form.handleSubmit(enable)}
      >
        <div className="flex flex-wrap gap-2">
          <Badge variant={runtime?.public_enabled ? "success" : "secondary"}>
            Public {runtime?.public_enabled ? "enabled" : "paused"}
          </Badge>
          <Badge variant={runtime?.personal_enabled ? "success" : "secondary"}>
            Personal {runtime?.personal_enabled ? "enabled" : "paused"}
          </Badge>
          {stopped && <Badge variant="destructive">Manager stopped</Badge>}
        </div>
        <p>
          Optional public AI uses the selected sponsor organization's credits.
          Built-in FAQ and signup links need no inference credits. Personal work
          uses each person's existing account billing and organization rules.
        </p>
        <FormField
          control={form.control}
          name="source"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Agent source</FormLabel>
              <FormControl>
                <select
                  {...field}
                  className="h-8 w-full rounded-lg border border-input bg-background px-3"
                >
                  <option value="nyxagent">
                    NyxAgent
                    {profiles.find((row) => row.id === selection)
                      ? ` · ${profiles.find((row) => row.id === selection)!.label}`
                      : ""}
                  </option>
                </select>
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <div className="grid gap-3 sm:grid-cols-2">
          {(
            [
              ["total_daily_credits", "Total public budget per day"],
              ["sender_daily_credits", "Free quota per person per day"],
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
                    <Input {...field} inputMode="decimal" />
                  </FormControl>
                  <p className="text-11 text-muted-foreground">Credits</p>
                  <FormMessage />
                </FormItem>
              )}
            />
          ))}
        </div>
        <p className="text-muted-foreground">
          Public tools can read the public FAQ and offer signup or linking. They
          cannot access anyone's account, services, memory or machines.
        </p>
        <p role="status" className="text-muted-foreground">
          {readiness.status === "ready"
            ? "Readiness was verified. Apply activates this deployment. A changed deployment ends existing public sessions so senders can start fresh."
            : reasons[readiness.reason!]}
        </p>
        {error && <ErrorBanner message={error} />}
        <div className="flex flex-wrap gap-2">
          <Button
            type="submit"
            variant="primary"
            isLoading={pending}
            disabled={stopped}
          >
            {runtime
              ? runtime.public_enabled
                ? form.formState.isDirty
                  ? "Save public settings"
                  : "Validate and apply public deployment"
                : "Enable public guide"
              : "Enable bot"}
          </Button>
        </div>
        <div className="space-y-3 rounded-lg border border-border p-3">
          <FormField
            control={form.control}
            name="profile"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Public NyxAgent profile</FormLabel>
                <FormControl>
                  <select
                    {...field}
                    className="h-8 w-full rounded-lg border border-input bg-background px-3"
                  >
                    {!profiles.some((row) => row.id === field.value) && (
                      <option value={field.value}>
                        {field.value
                          ? `${field.value} (unverified draft)`
                          : "Use the advertised public default"}
                      </option>
                    )}
                    {profiles.map((row) => (
                      <option key={row.id} value={row.id}>
                        {row.label}
                      </option>
                    ))}
                  </select>
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <Button
            type="button"
            variant="outline"
            disabled={pending || stopped}
            onClick={() => void discover()}
          >
            Refresh public profiles
          </Button>
          <FormField
            control={form.control}
            name="sponsor_owner_user_id"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Sponsor organization</FormLabel>
                <FormControl>
                  <select
                    {...field}
                    className="h-8 w-full rounded-lg border border-input bg-background px-3"
                  >
                    <option value="">Choose an organization</option>
                    {field.value &&
                      !targets.data?.orgs.some(
                        (org) => org.id === field.value,
                      ) && (
                        <option value={field.value}>
                          {field.value} (authorization rechecked on save)
                        </option>
                      )}
                    {targets.data?.orgs.map((org) => (
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
          <p>
            Profile discovery and readiness may consume sponsor credits. Public
            limits count admitted maximum costs for forwarded work, while
            billing settles actual usage. Public quotas do not limit personal
            chats.
          </p>
          <p>
            Rate, concurrency and execution limits are validated by the server.
            This API does not offer editable runtime bounds.
          </p>
        </div>
      </form>
    </Form>
  );
}
