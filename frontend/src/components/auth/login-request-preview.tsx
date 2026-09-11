import { ChevronDown, Info } from "lucide-react";
import { formatAuthDeviceUserCodeInput, type PreviewAuthDeviceResponse } from "@/schemas/auth-device";
import { formatAuthDeviceRelativeTime, formatWebAuthDeviceRemaining } from "@/lib/auth-device-time";
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
}: {
  readonly preview: PreviewAuthDeviceResponse;
  readonly remainingSeconds: number | null;
  readonly userCode: string;
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
  const deviceDescription =
    preview.client_label && preview.client_model
      ? `${preview.client_label} · ${preview.client_model}`
      : (preview.client_label ?? preview.client_model ?? "Not provided");
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
    <section
      className="border-t border-border/50 pt-4"
      aria-label="Request details"
    >
      <div className="divide-y divide-border/30 overflow-hidden rounded-xl border border-border/50 bg-overlay/30">
        <div data-sensitive>
          <ApprovalDetailRow label="User code" value={formatAuthDeviceUserCodeInput(userCode)} mono />
          <p className="px-4 pb-3 text-[12px] text-muted-foreground">
            Confirm this matches the code shown on the requesting device or terminal. Reject if it does not match.
          </p>
        </div>
        {/*
          A signal whose "good" state can be produced by an attacker choosing
          what to send must never render as a positive assurance. Origin is a
          forgeable header on this public endpoint, and even a first-party proof
          would not stop an attacker from copying a genuine QR, so only negative
          origin states are informative.
        */}
        {originValue ? (
          <ApprovalDetailRow
            label="Started from"
            value={originValue}
            tone={originTone}
          />
        ) : null}
        <ApprovalDetailRow label="Status" value={capitalize(preview.status)} />
        <ApprovalDetailRow
          label="Requester"
          value={
            verifiedIp && preview.client_ip
              ? preview.client_ip
              : unverifiedIp
                ? "Not verified"
                : "IP unavailable on this deployment"
          }
          mono={verifiedIp && preview.client_ip !== null}
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
        {unverifiedIp && preview.client_ip ? (
          <ApprovalDetailRow
            label="Reported IP"
            value={`${preview.client_ip} · unverified`}
            mono
          />
        ) : null}
        <ApprovalDetailRow
          label="Requested"
          value={`${formatAuthDeviceRelativeTime(preview.initiated_at)} · ${formatAbsoluteDateTime(preview.initiated_at)}`}
        />
        <ApprovalDetailRow
          label="Expires in"
          value={
            expired
              ? "Expired"
              : formatWebAuthDeviceRemaining(remainingSeconds ?? 0)
          }
          tone={expiryTone}
        />
        <ApprovalDetailRow label="Reported device" value={deviceDescription} />
        <ApprovalDetailRow label="Reported client" value={appDescription} />
        <ApprovalDetailRow
          label="Platform"
          value={preview.client_platform ?? "Not identified"}
        />
        <ApprovalDetailRow
          label="Form factor"
          value={
            preview.client_form_factor
              ? capitalize(preview.client_form_factor)
              : "Not reported"
          }
        />
        <ApprovalDetailRow
          label="Timezone"
          value={timezoneValue}
          tone={timezoneTone}
        />
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
        <details className="group px-4 py-2.5">
          <summary className="flex cursor-pointer list-none items-center justify-between gap-3 text-[12px] text-muted-foreground">
            Raw user agent
            <ChevronDown className="size-3.5 transition-transform group-open:rotate-180" />
          </summary>
          <p className="mt-2 break-all font-mono text-[11px] leading-relaxed text-foreground">
            {preview.client_user_agent ?? "Not provided"}
          </p>
        </details>
      </div>
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
