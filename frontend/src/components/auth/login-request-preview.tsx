import { ChevronDown, Info } from "lucide-react";
import {
  formatAuthDeviceUserCodeInput,
  type PreviewAuthDeviceResponse,
} from "@/schemas/auth-device";
import {
  formatAuthDeviceRelativeTime,
  formatWebAuthDeviceRemaining,
} from "@/lib/auth-device-time";
import { cn } from "@/lib/utils";

export function LoginDeviceShell({
  children,
}: {
  readonly children: React.ReactNode;
}) {
  return (
    <main className="flex min-h-dvh items-start justify-center bg-background px-4 py-8 text-foreground sm:items-center sm:py-10">
      <div className="flex w-full max-w-xl flex-col gap-5">{children}</div>
    </main>
  );
}

export function ApprovalCaution() {
  return (
    <div className="flex items-start gap-2 px-1 text-[12px] leading-relaxed text-muted-foreground">
      <Info className="mt-0.5 size-4 shrink-0" />
      <p>
        Only approve if you started this sign-in.{" "}
        <span className="font-medium text-destructive">
          If anything looks unfamiliar, reject it.
        </span>
      </p>
    </div>
  );
}

export function PreviewPanel({
  preview,
  remainingSeconds,
  userCode,
  requestedProfile,
  detailsOpen,
  onDetailsOpenChange,
}: {
  readonly preview: PreviewAuthDeviceResponse;
  readonly remainingSeconds: number | null;
  readonly userCode: string;
  readonly requestedProfile?: string | null;
  readonly detailsOpen: boolean;
  readonly onDetailsOpenChange: (open: boolean) => void;
}) {
  const expired = remainingSeconds === 0;
  const verifiedIp = preview.client_ip_attribution === "verified";
  const unverifiedIp = preview.client_ip_attribution === "unverified";
  const networkRelation =
    preview.network_relation ??
    (preview.same_ip_as_viewer === true
      ? "same_ip"
      : preview.same_ip_as_viewer === false
        ? "different_ip"
        : null);
  const localTimezone = browserTimezone();
  const reportedTimezoneDiffers =
    preview.client_timezone !== null &&
    localTimezone !== null &&
    preview.client_timezone.toLowerCase() !== localTimezone.toLowerCase();
  const timezoneDifferences = [
    reportedTimezoneDiffers ? "this device" : null,
    preview.client_timezone !== null &&
    preview.client_timezone_matches_ip === false
      ? "IP location"
      : null,
  ].filter((value): value is string => value !== null);
  const timezoneValue = preview.client_timezone
    ? timezoneDifferences.length > 0
      ? `${preview.client_timezone} · differs from ${timezoneDifferences.join(" and ")}`
      : preview.client_timezone
    : "Not reported";
  const location = formatVerifiedLocation(preview);
  const screenDescription = formatScreenDescription(preview);
  const originValue = initiatingOriginValue(preview);
  const appDescription =
    preview.client_app ??
    (preview.client_kind === "unknown"
      ? "Not identified"
      : `${preview.client_kind[0]?.toUpperCase()}${preview.client_kind.slice(1)} client`);
  // Upstream's caution sentence owns one accent. Keep at most one additional
  // value tint, prioritizing the security signal over recognition details.
  const originTone: DetailValueTone = originValue ? "danger" : "default";
  const timezoneTone: DetailValueTone =
    !originValue && timezoneDifferences.length > 0 ? "warning" : "default";
  const expiryTone: DetailValueTone =
    originValue || timezoneDifferences.length > 0
      ? "default"
      : expired
        ? "danger"
        : remainingSeconds !== null && remainingSeconds <= 60
          ? "warning"
          : "default";

  return (
    <section className="space-y-3" aria-label="Request details">
      <div
        data-sensitive
        className="rounded-lg border border-border bg-background px-4 py-5 text-center"
      >
        <span className="sr-only">User code: </span>
        <p className="font-mono text-[28px] font-medium tracking-widest">
          {formatAuthDeviceUserCodeInput(userCode)}
        </p>
        <p
          className={cn(
            "mt-2 text-[11px] text-muted-foreground",
            expiryTone === "danger" && "text-destructive",
            expiryTone === "warning" && "text-warning",
          )}
        >
          {expired
            ? "Expired"
            : remainingSeconds === null
              ? "Checking expiry…"
              : `Expires in ${formatWebAuthDeviceRemaining(remainingSeconds)}`}
        </p>
      </div>
      <div className="flex flex-wrap items-center justify-center gap-x-2 gap-y-1 text-[12px]">
        <span className="text-muted-foreground">Requester</span>
        <span className="min-w-0 break-words font-medium">
          {preview.client_label ?? preview.client_app ?? "Requesting device"}
        </span>
        <span className="font-mono text-[11px] text-muted-foreground">
          {verifiedIp && preview.client_ip
            ? preview.client_ip
            : unverifiedIp
              ? "IP not verified"
              : "IP unavailable"}
        </span>
      </div>
      {/* Reported origin can be forged: show negative signals, never a trust badge. */}
      {originValue && (
        <div className="rounded-lg border border-destructive/20 bg-destructive/5">
          <ApprovalDetailRow
            label="Started from"
            value={originValue}
            tone={originTone}
          />
        </div>
      )}
      {timezoneDifferences.length > 0 && (
        <ApprovalDetailRow
          label="Timezone"
          value={timezoneValue}
          tone={timezoneTone}
        />
      )}
      <details
        className="group rounded-lg border border-border/50"
        open={detailsOpen}
        onToggle={(event) => onDetailsOpenChange(event.currentTarget.open)}
      >
        <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-2.5 text-[12px] text-muted-foreground hover:text-foreground">
          Request details
          <ChevronDown
            aria-hidden="true"
            className="size-3.5 transition-transform group-open:rotate-180"
          />
        </summary>
        <div className="divide-y divide-border/30 border-t border-border/50">
          <ApprovalDetailRow
            label="Status"
            value={capitalize(preview.status)}
          />
          <ApprovalDetailRow
            label="Location"
            value={verifiedIp ? (location ?? "Not available") : "Not available"}
          />
          <ApprovalDetailRow
            label="Network"
            value={
              verifiedIp
                ? formatNetworkRelation(networkRelation)
                : "Not available"
            }
          />
          {unverifiedIp && preview.client_ip && (
            <ApprovalDetailRow
              label="Reported IP"
              value={`${preview.client_ip} · unverified`}
              mono
            />
          )}
          <ApprovalDetailRow
            label="Requested"
            value={`${formatAuthDeviceRelativeTime(preview.initiated_at)} · ${formatAbsoluteDateTime(preview.initiated_at)}`}
          />
          <ApprovalDetailRow
            label="Reported model"
            value={preview.client_model ?? "Not provided"}
          />
          <ApprovalDetailRow label="Reported client" value={appDescription} />
          <ApprovalDetailRow
            label="Platform"
            value={preview.client_platform ?? "Not identified"}
          />
          {requestedProfile && (
            <ApprovalDetailRow
              label="Requested profile"
              value={requestedProfile}
            />
          )}
          <ApprovalDetailRow
            label="Form factor"
            value={
              preview.client_form_factor
                ? capitalize(preview.client_form_factor)
                : "Not reported"
            }
          />
          {timezoneDifferences.length === 0 && (
            <ApprovalDetailRow label="Timezone" value={timezoneValue} />
          )}
          <ApprovalDetailRow
            label="Locale"
            value={preview.client_locale ?? "Not reported"}
          />
          <ApprovalDetailRow
            label="Screen"
            value={screenDescription ?? "Not reported"}
          />
          <ApprovalDetailRow
            label="Processor"
            value={
              preview.client_hardware_concurrency === null
                ? "Not reported"
                : `${preview.client_hardware_concurrency} logical processors`
            }
          />
          <ApprovalDetailRow
            label="Memory"
            value={
              preview.client_device_memory === null
                ? "Not reported"
                : `${preview.client_device_memory} GB`
            }
          />
          <ApprovalDetailRow
            label="Raw user agent"
            value={preview.client_user_agent ?? "Not provided"}
            mono
          />
        </div>
      </details>
    </section>
  );
}

function initiatingOriginValue(
  preview: PreviewAuthDeviceResponse,
): string | null {
  if (
    preview.initiating_origin_status === "absent" ||
    preview.initiating_origin_status === "matched"
  ) {
    return null;
  }
  if (preview.initiating_origin_status === "mismatched") {
    return originHost(preview.initiating_origin) ?? "Another site";
  }
  return preview.initiating_origin_status === "non_http"
    ? "Non-HTTP origin"
    : "Malformed origin";
}

function originHost(origin: string | null): string | null {
  if (!origin) return null;
  try {
    return new URL(origin).host || null;
  } catch {
    return null;
  }
}

function browserTimezone(): string | null {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

function formatVerifiedLocation(
  preview: PreviewAuthDeviceResponse,
): string | null {
  const locality = [preview.client_city, preview.client_region]
    .filter((value): value is string => Boolean(value))
    .join(", ");
  const place =
    locality && preview.client_country
      ? `${locality} (${preview.client_country})`
      : locality || preview.client_country || preview.client_continent;
  if (place && preview.client_ip_timezone) {
    return `${place} · ${preview.client_ip_timezone}`;
  }
  if (place) return place;
  return preview.client_ip_timezone
    ? `IP timezone: ${preview.client_ip_timezone}`
    : null;
}

function formatNetworkRelation(
  relation:
    | "same_ip"
    | "same_network"
    | "different_network"
    | "different_ip"
    | null,
): string {
  if (relation === "same_ip") return "Same IP as this device";
  if (relation === "same_network") return "Same network as this device";
  if (relation === "different_network") return "Different network";
  if (relation === "different_ip") return "Different IP";
  return "Not available";
}

function formatScreenDescription(
  preview: PreviewAuthDeviceResponse,
): string | null {
  if (
    preview.client_screen_width === null ||
    preview.client_screen_height === null
  ) {
    return null;
  }
  const ratio =
    preview.client_device_pixel_ratio === null
      ? ""
      : ` at ${preview.client_device_pixel_ratio}x`;
  return `${preview.client_screen_width} x ${preview.client_screen_height} CSS px${ratio}`;
}

function capitalize(value: string): string {
  return `${value[0]?.toUpperCase() ?? ""}${value.slice(1)}`;
}

type DetailValueTone = "default" | "warning" | "danger";

function ApprovalDetailRow({
  label,
  value,
  mono = false,
  tone = "default",
}: {
  readonly label: string;
  readonly value: string;
  readonly mono?: boolean;
  readonly tone?: DetailValueTone;
}) {
  return (
    <div className="flex items-start justify-between gap-4 px-4 py-2.5 text-[12px]">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span
        className={cn(
          "min-w-0 break-words text-right text-foreground",
          mono ? "font-mono text-[11px]" : "font-medium",
          tone === "warning" && "text-warning",
          tone === "danger" && "text-destructive",
        )}
      >
        {value}
      </span>
    </div>
  );
}

function formatAbsoluteDateTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "Unknown";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "medium",
  }).format(timestamp);
}
