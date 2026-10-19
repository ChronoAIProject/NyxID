import { channelPlatformName } from "@/lib/assistant/nyxbot-labels";
import { useState } from "react";
import { useAdminPlatformTargets } from "@/hooks/use-platform-onboarding";
import {
  useNativeEntryConversion,
  useNativeEntryTransports,
  useNativeGatewayInspection,
  useNativeLifecycleReview,
} from "@/hooks/use-native-platform-entry";
import { useAuthStore } from "@/stores/auth-store";
import type {
  NativeSetupRecovery,
  NativeLifecycleReview,
  NativeTransport,
} from "@/schemas/native-platform-entry";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";

export function NativePlatformEntrySetup({
  profile,
  onPrepared,
}: {
  profile?: PlatformProfile;
  onPrepared?: (profile: PlatformProfile) => void;
}) {
  const actor = useAuthStore((s) => s.user?.id);
  const [showFunding, setShowFunding] = useState(false);
  const targets = useAdminPlatformTargets(showFunding);
  const capabilities = useNativeEntryTransports();
  const conversion = useNativeEntryConversion();
  const inspection = useNativeGatewayInspection();
  const lifecycle = useNativeLifecycleReview();
  const [lifecycleReview, setLifecycleReview] = useState<{
    selection: string;
    value: NativeLifecycleReview;
  }>();
  const [lifecycleConfirmed, setLifecycleConfirmed] = useState<string>();
  const [connectionId, setConnectionId] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [botId, setBotId] = useState(profile?.channel_bot_id ?? "");
  const [fundingId, setFundingId] = useState("");
  const [transport, setTransport] = useState<NativeTransport | "">(
    profile?.transport ?? "",
  );
  const [review, setReview] = useState<{
    selection: string;
    value: NativeSetupRecovery;
  }>();
  const [confirmed, setConfirmed] = useState<string>();
  const bots = targets.data?.bots.filter((bot) =>
    capabilities.data?.some((item) => item.platform === bot.platform),
  );
  const bot = profile
    ? {
        id: profile.channel_bot_id,
        platform: profile.platform ?? "telegram",
        label: profile.display_name ?? profile.manager_bot_id,
      }
    : bots?.find((item) => item.id === botId);
  const transports =
    capabilities.data?.find((item) => item.platform === bot?.platform)
      ?.transports ??
    profile?.supported_transports ??
    [];
  const selection = JSON.stringify([
    actor,
    botId,
    fundingId,
    transport,
    connectionId,
    publicKey,
    profile?.generation,
  ]);
  const reviewedLifecycle =
    lifecycleReview?.selection === selection
      ? lifecycleReview.value
      : undefined;
  const lifecycleKey = reviewedLifecycle
    ? `${selection}:${reviewedLifecycle.snapshot_digest}`
    : undefined;
  const current = review?.selection === selection ? review.value : undefined;
  const reviewKey = current
    ? `${selection}:${JSON.stringify(current)}`
    : undefined;
  const available =
    !!actor && !!bot && !!transport && transports.includes(transport);
  const busy =
    conversion.isPending || inspection.isPending || lifecycle.isPending;
  const keepReview = (value: NativeSetupRecovery) => {
    setReview({ selection, value });
    setConfirmed(undefined);
  };
  const convert = (recovery?: NativeSetupRecovery, update = false) => {
    if (!available || !transport) return;
    conversion.mutate(
      {
        channel_bot_id: botId,
        ...(fundingId ? { funding_owner_user_id: fundingId } : {}),
        transport,
        ...(recovery ? { recovery } : {}),
        ...(update && reviewedLifecycle
          ? {
              lifecycle_review: reviewedLifecycle,
              ...(connectionId
                ? { replacement_connection_id: connectionId }
                : {}),
              ...(publicKey ? { public_key: publicKey } : {}),
            }
          : {}),
      },
      {
        onSettled: () => {
          setConnectionId("");
          setPublicKey("");
          setLifecycleReview(undefined);
          setLifecycleConfirmed(undefined);
        },
        onSuccess: (result) => {
          if (result.status === "recovery") keepReview(result.review);
          else {
            setReview(undefined);
            setConfirmed(undefined);
            onPrepared?.(result.profile);
            if (!profile) {
              setBotId("");
              setTransport("");
            }
          }
        },
      },
    );
  };
  return (
    <DetailSection
      title={
        profile
          ? "Review native entry update"
          : "Shared entry for another chat platform"
      }
    >
      <div className="space-y-4 p-4 text-12">
        <p>
          {profile
            ? "Review this entry’s current credentials and transport before updating. Updating pauses both access modes and revokes prior chat authority. Owner routing remains closed; explicitly enable access after verification."
            : "Conversion permanently closes owner routing and starts the shared entry paused. Select the delivery transport explicitly. Discord questions use /nyx ask; Slack and other private-message controls use their supported chat surface."}
        </p>
        <label className="block space-y-1">
          Chat channel
          <select
            aria-label="Native chat channel"
            className="h-8 w-full rounded-lg border border-input bg-background px-3"
            value={botId}
            disabled={!!profile}
            onChange={(event) => {
              setBotId(event.target.value);
              setTransport("");
            }}
          >
            <option value="">Choose a channel</option>
            {(profile && bot ? [bot] : bots)?.map((item) => (
              <option key={item.id} value={item.id}>
                {item.label} · {channelPlatformName(item.platform)}
              </option>
            ))}
          </select>
        </label>
        <label className="block space-y-1">
          Delivery transport
          <select
            aria-label="Native delivery transport"
            className="h-8 w-full rounded-lg border border-input bg-background px-3"
            value={transport}
            onChange={(event) =>
              setTransport(event.target.value as NativeTransport | "")
            }
          >
            <option value="">Choose a transport</option>
            {transports.map((item) => (
              <option key={item} value={item}>
                {item === "direct"
                  ? "Direct native delivery"
                  : "Gateway transport"}
              </option>
            ))}
          </select>
        </label>
        {bot && !transports.includes("gateway") && (
          <p>
            Gateway transport is unavailable for {bot.platform}. Select direct
            native delivery to continue.
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button disabled={!available || busy} onClick={() => convert()}>
            {profile
              ? "Continue reserved native setup"
              : "Convert native entry and pause"}
          </Button>
          <Button
            disabled={!available || busy}
            onClick={() =>
              lifecycle.mutate(botId, {
                onSuccess: (value) => {
                  setLifecycleReview({ selection, value });
                  setLifecycleConfirmed(undefined);
                },
              })
            }
          >
            {profile
              ? "Review current native entry"
              : "Review failed or changed native setup"}
          </Button>
          {transport === "gateway" && (
            <Button
              disabled={!available || busy}
              onClick={() =>
                inspection.mutate(botId, { onSuccess: keepReview })
              }
            >
              Inspect retained gateway setup
            </Button>
          )}
        </div>
        <details onToggle={(event) => setShowFunding(event.currentTarget.open)}>
          <summary>Replace reviewed credentials or setup funding</summary>
          <label className="block space-y-1">
            Optional funding organization
            <select
              aria-label="Native funding organization"
              value={fundingId}
              onChange={(event) => setFundingId(event.target.value)}
              className="h-8 w-full rounded-lg border border-input bg-background px-3"
            >
              <option value="">
                No organization required for FAQ and personal access
              </option>
              {targets.data?.orgs.map((org) => (
                <option key={org.id} value={org.id}>
                  {org.display_name ?? org.slug}
                </option>
              ))}
            </select>
          </label>
          <p>
            Use a connected credential owned by this channel’s existing owner.
            Leaving these fields empty retains its current credential selection.
          </p>
          <label>
            Replacement OAuth connection
            <Input
              aria-label="Native replacement connection"
              value={connectionId}
              onChange={(event) => setConnectionId(event.target.value)}
            />
          </label>
          {bot?.platform === "discord" && (
            <label>
              Discord public verification key
              <Input
                aria-label="Native verification public key"
                value={publicKey}
                onChange={(event) => setPublicKey(event.target.value)}
              />
            </label>
          )}
        </details>
        {reviewedLifecycle && (
          <div className="space-y-3 rounded-lg border border-border p-3">
            <p>
              This review matches the retained entry. Confirming reserves a
              paused update under your current administrator and funding access.
              A failed provider check keeps the reservation available for review
              and retry.
            </p>
            <div className="flex items-center gap-2">
              <Checkbox
                id={`native-update-review-${botId}`}
                checked={lifecycleConfirmed === lifecycleKey}
                onCheckedChange={(checked) =>
                  setLifecycleConfirmed(checked ? lifecycleKey : undefined)
                }
              />
              <label htmlFor={`native-update-review-${botId}`}>
                I reviewed this entry and its selected replacement credentials.
              </label>
            </div>
            <Button
              disabled={
                !available ||
                busy ||
                !lifecycleKey ||
                lifecycleConfirmed !== lifecycleKey
              }
              onClick={() => convert(undefined, true)}
            >
              Update reviewed native entry and pause
            </Button>
          </div>
        )}
        {current && (
          <div className="space-y-3 rounded-lg border border-border p-3">
            <p>
              {current.kind === "gateway"
                ? "Review the retained gateway record before retrying its original setup. The record, request and credentials remain bound to this attempt."
                : "Review the observed Discord command before recovering this installation."}
            </p>
            {current.kind === "gateway" && (
              <p>
                {current.remote_ready
                  ? "The retained remote record is ready."
                  : current.remote_channel_id
                    ? "The retained remote reservation is incomplete."
                    : "The retained remote record was not found."}
              </p>
            )}
            <details>
              <summary>Setup identifiers</summary>
              <p className="break-all">
                {current.kind === "gateway"
                  ? current.record_id
                  : current.application_id}{" "}
                · {current.attempt_id}
              </p>
            </details>
            <div className="flex items-center gap-2">
              <Checkbox
                id="native-setup-review"
                checked={confirmed === reviewKey}
                onCheckedChange={(checked) =>
                  setConfirmed(checked ? reviewKey : undefined)
                }
              />
              <label htmlFor="native-setup-review">
                I reviewed this exact retained setup.
              </label>
            </div>
            <Button
              disabled={
                !available || busy || !reviewKey || confirmed !== reviewKey
              }
              onClick={() => convert(current)}
            >
              Retry reviewed native setup
            </Button>
          </div>
        )}
        {(capabilities.error || targets.error) && (
          <ErrorBanner message="Could not load native channels and supported transports." />
        )}
        {conversion.error && <ErrorBanner message={conversion.error.message} />}
        {inspection.error && <ErrorBanner message={inspection.error.message} />}
        {lifecycle.error && <ErrorBanner message={lifecycle.error.message} />}
      </div>
    </DetailSection>
  );
}
