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
  public_turn_failed:
    "The public turn failed. Try again to start a fresh turn.",
  mode_disabled: "Public access is not available on this server.",
  manager_unavailable: "The manager is unavailable.",
  emergency_stopped:
    "The manager is stopped. Explicitly clear the stop before enabling access.",
  personal_disabled: "Personal access is paused.",
  public_disabled: "Public access is paused.",
  owner_unavailable: "The owner account is unavailable.",
  transport_unavailable: "Chat transport is unavailable.",
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
    <RuntimeForm
      key={`${profile.generation}:${runtime?.manager_authority_epoch ?? 0}:${runtime?.personal_control_revision ?? 0}:${runtime?.public_policy_revision ?? 0}`}
      profile={profile}
      value={query.data}
    />
  );
}
function RuntimeForm({
  profile,
  value,
}: {
  profile: PlatformProfile;
  value: ManagerRuntime;
}) {
  const runtime = value.runtime;
  const policy = runtime?.public_policy;
  const mutation = useManagerRuntimeAction(profile.id);
  const targets = useAdminPlatformTargets();
  const [profiles, setProfiles] = useState<{ id: string; label: string }[]>([]);
  const [readiness, setReadiness] = useState(value.public_readiness);
  const [error, setError] = useState<string>();
  const [clearConfirmed, setClearConfirmed] = useState(false);
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
        policy?.sponsor_owner_user_id ?? profile.funding_owner_user_id,
    },
  });
  const expected = runtimePreconditions(profile.generation, runtime);
  const pending = mutation.isPending;
  const stopped = runtime?.emergency_stopped === true || !!profile.deleted_at;
  const selection = form.watch("profile");
  function publicBody(values: PublicSetup) {
    // Sponsor is selected at manager setup. An Advanced change is explicit.
    const { sponsor_owner_user_id, ...rest } = values;
    return {
      ...expected,
      public_policy: {
        ...rest,
        ...(rest.profile ? {} : { profile: undefined }),
        ...(sponsor_owner_user_id !==
        (policy?.sponsor_owner_user_id ?? profile.funding_owner_user_id)
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
            : "Choose an advertised public profile in Advanced. This runtime has no public default.",
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
  async function control(
    action: "personal-enable" | "personal-pause" | "public-pause" | "clear",
  ) {
    setError(undefined);
    try {
      await mutation.mutateAsync({
        action: action === "clear" ? "clear-emergency-stop" : "update",
        body: {
          ...expected,
          ...(action === "personal-enable"
            ? {
                personal_enabled: true,
                ...(!runtime ? { public_enabled: false } : {}),
              }
            : {}),
          ...(action === "personal-pause" ? { personal_enabled: false } : {}),
          ...(action === "public-pause" ? { public_enabled: false } : {}),
        },
      });
    } catch {
      setError(
        "The control could not be updated. Reload settings and review the current state before retrying.",
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
          Public FAQ and signup use the sponsor organization's credits. Linked
          personal chats use each person's existing NyxBot access and normal
          billing.
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
        <p>
          Sponsor organization:{" "}
          <span className="font-mono">
            {form.watch("sponsor_owner_user_id") ||
              "Select an authorized organization in Advanced"}
          </span>
        </p>
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
          {runtime?.public_enabled && (
            <Button
              type="button"
              variant="outline"
              disabled={pending}
              onClick={() => void control("public-pause")}
            >
              Pause public guide
            </Button>
          )}
        </div>
        <details className="space-y-3 rounded-lg border border-border p-3">
          <summary className="cursor-pointer font-medium">Advanced</summary>
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
          <div className="space-y-2 border-t border-border pt-3">
            <p className="font-medium">Personal Telegram access</p>
            <p>
              Enable personal access independently while public access is
              unavailable. Each person still signs in, reviews consent and
              confirms in the same private chat.
            </p>
            <Button
              type="button"
              variant="outline"
              disabled={pending || stopped}
              onClick={() =>
                void control(
                  runtime?.personal_enabled
                    ? "personal-pause"
                    : "personal-enable",
                )
              }
            >
              {runtime?.personal_enabled
                ? "Pause personal access"
                : "Enable personal access only"}
            </Button>
          </div>
          {runtime?.emergency_stopped && (
            <div className="space-y-2 border-t border-border pt-3">
              <label className="flex items-start gap-2">
                <Checkbox
                  checked={clearConfirmed}
                  onCheckedChange={(checked) =>
                    setClearConfirmed(checked === true)
                  }
                />
                <span>
                  I reviewed the manager stop and want to clear it. This does
                  not enable either access lane.
                </span>
              </label>
              <Button
                type="button"
                variant="destructive"
                disabled={!clearConfirmed || pending}
                onClick={() => void control("clear")}
              >
                Clear manager emergency stop
              </Button>
            </div>
          )}
        </details>
      </form>
    </Form>
  );
}
