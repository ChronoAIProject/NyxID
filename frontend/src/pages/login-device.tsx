import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  CheckCircle2,
  ChevronDown,
  CircleX,
  Info,
  ShieldCheck,
  ShieldX,
} from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useApproveAuthDevice,
  useDenyAuthDevice,
  usePreviewAuthDevice,
} from "@/hooks/use-auth-device";
import {
  formatAuthDeviceUserCodeInput,
  friendlyAuthDeviceErrorMessage,
  friendlyAuthDeviceStatusMessage,
  type PreviewAuthDeviceResponse,
  userCodeSchema,
} from "@/schemas/auth-device";
import {
  formatAuthDeviceRelativeTime,
  formatWebAuthDeviceRemaining,
  resolveAuthDeviceDeadlineMs,
  secondsUntilAuthDeviceDeadline,
} from "@/lib/auth-device-time";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";

const VALID_CODE_LENGTH = 9;
const CLICK_THROTTLE_MS = 750;

export function LoginDevicePage() {
  const navigate = useNavigate();
  const { isAuthenticated, isLoading } = useAuthStore();
  const [userCode, setUserCode] = useState("");
  const [decision, setDecision] = useState<"approved" | "denied" | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [clockMs, setClockMs] = useState(() => Date.now());
  const lastClickAtRef = useRef(0);
  const normalizedCode = useMemo(() => {
    const parsed = userCodeSchema.safeParse(userCode);
    return parsed.success ? parsed.data : null;
  }, [userCode]);
  const preview = usePreviewAuthDevice();
  const [previewDeadline, setPreviewDeadline] = useState<number | null>(() =>
    preview.data
      ? resolveAuthDeviceDeadlineMs(
          preview.data.expires_at,
          preview.data.seconds_remaining,
        )
      : null,
  );
  const approve = useApproveAuthDevice();
  const deny = useDenyAuthDevice();
  const isDecisionPending = approve.isPending || deny.isPending;
  const previewStatusMessage = preview.data
    ? friendlyAuthDeviceStatusMessage(preview.data.status)
    : null;
  const step: "enter-code" | "review" | "terminal" = decision
    ? "terminal"
    : preview.data
      ? "review"
      : "enter-code";
  const previewRemaining =
    previewDeadline === null
      ? null
      : secondsUntilAuthDeviceDeadline(previewDeadline, clockMs);
  const locallyExpired = previewRemaining === 0;

  useEffect(() => {
    if (isLoading || isAuthenticated) return;
    void navigate({ to: "/login", search: { return_to: "/login/device" } });
  }, [isAuthenticated, isLoading, navigate]);

  useEffect(() => {
    if (previewDeadline === null) return;
    const interval = window.setInterval(() => {
      const now = Date.now();
      setClockMs(now);
      if (now >= previewDeadline) window.clearInterval(interval);
    }, 1000);
    return () => window.clearInterval(interval);
  }, [previewDeadline]);

  function withinCooldown(): boolean {
    const now = Date.now();
    if (now - lastClickAtRef.current < CLICK_THROTTLE_MS) return true;
    lastClickAtRef.current = now;
    return false;
  }

  function resetToEnterCode() {
    preview.reset();
    approve.reset();
    deny.reset();
    setSubmitError(null);
    setPreviewDeadline(null);
  }

  async function handleContinue() {
    if (!normalizedCode || preview.isPending || withinCooldown()) return;
    setSubmitError(null);
    try {
      const result = await preview.mutateAsync(normalizedCode);
      const now = Date.now();
      setClockMs(now);
      setPreviewDeadline(
        resolveAuthDeviceDeadlineMs(
          result.expires_at,
          result.seconds_remaining,
          now,
        ),
      );
    } catch (error) {
      setSubmitError(friendlyAuthDeviceErrorMessage(error));
    }
  }

  async function handleApprove() {
    if (!normalizedCode || isDecisionPending || withinCooldown()) return;
    setSubmitError(null);
    try {
      await approve.mutateAsync(normalizedCode);
      setDecision("approved");
    } catch (error) {
      setSubmitError(friendlyAuthDeviceErrorMessage(error));
    }
  }

  async function handleReject() {
    if (!normalizedCode || isDecisionPending || withinCooldown()) return;
    setSubmitError(null);
    try {
      await deny.mutateAsync(normalizedCode);
      setDecision("denied");
    } catch (error) {
      setSubmitError(friendlyAuthDeviceErrorMessage(error));
    }
  }

  if (isLoading || !isAuthenticated) {
    return (
      <LoginDeviceShell>
        <Card className="border-border/50">
          <CardContent className="p-4">
            <Skeleton className="h-52 w-full" />
          </CardContent>
        </Card>
      </LoginDeviceShell>
    );
  }

  return (
    <LoginDeviceShell>
      <header className="flex flex-col gap-3 text-center">
        <div className="flex justify-center">
          <NyxidIcon className="h-10 w-10" />
        </div>
        <div className="space-y-1">
          <h1 className="text-[22px] font-bold leading-tight tracking-tight text-foreground sm:text-[28px]">
            Approve a device login
          </h1>
          <p className="mx-auto max-w-md text-[12px] text-muted-foreground">
            Review the one-time code shown on the requesting device.
          </p>
        </div>
      </header>

      {decision ? (
        <TerminalPanel decision={decision} />
      ) : (
        <Card className="border-border/50">
          <CardHeader>
            <CardTitle>Device login request</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <div className="space-y-2">
              <label
                className="text-[10px] font-medium uppercase tracking-[1.5px] text-text-tertiary"
                htmlFor="auth-device-code"
              >
                User code
              </label>
              <Input
                id="auth-device-code"
                autoComplete="one-time-code"
                className="h-14 text-center font-mono text-[22px] tracking-[0.16em]"
                inputMode="text"
                maxLength={VALID_CODE_LENGTH}
                placeholder="ABCD-EFGH"
                value={userCode}
                disabled={
                  step === "review" || preview.isPending || isDecisionPending
                }
                onChange={(event) => {
                  setSubmitError(null);
                  setDecision(null);
                  resetToEnterCode();
                  setUserCode(
                    formatAuthDeviceUserCodeInput(event.target.value),
                  );
                }}
              />
            </div>

            {submitError ? <ErrorBanner message={submitError} /> : null}
            {!normalizedCode && userCode.replace("-", "").length === 8 ? (
              <ErrorBanner message="Enter an 8-character code using A-H, J-K, M-N, P-T, and V-Z." />
            ) : null}
            {preview.isError ? (
              <ErrorBanner
                message={friendlyAuthDeviceErrorMessage(preview.error)}
              />
            ) : null}

            {step === "review" && preview.data ? (
              <>
                <PreviewPanel
                  preview={preview.data}
                  remainingSeconds={previewRemaining}
                />
                <ApprovalCaution />
              </>
            ) : null}
            {previewStatusMessage ? (
              <ErrorBanner message={previewStatusMessage} />
            ) : null}

            <div className="flex flex-col gap-2 sm:flex-row sm:justify-end">
              <Button
                type="button"
                variant="outline"
                disabled={preview.isPending || isDecisionPending}
                onClick={() => void navigate({ to: "/dashboard" })}
              >
                Cancel
              </Button>
              {step === "enter-code" ? (
                <Button
                  type="button"
                  variant="primary"
                  disabled={!normalizedCode || preview.isPending}
                  isLoading={preview.isPending}
                  onClick={() => void handleContinue()}
                >
                  <ButtonIcon variant="primary">
                    <ShieldCheck />
                  </ButtonIcon>
                  Continue
                </Button>
              ) : step === "review" &&
                preview.data?.status === "pending" &&
                !locallyExpired ? (
                <>
                  <Button
                    type="button"
                    variant="destructive"
                    disabled={isDecisionPending}
                    isLoading={deny.isPending}
                    onClick={() => void handleReject()}
                  >
                    <ButtonIcon variant="destructive">
                      <ShieldX />
                    </ButtonIcon>
                    Reject
                  </Button>
                  <Button
                    type="button"
                    variant="primary"
                    disabled={isDecisionPending}
                    isLoading={approve.isPending}
                    onClick={() => void handleApprove()}
                  >
                    <ButtonIcon variant="primary">
                      <ShieldCheck />
                    </ButtonIcon>
                    Approve
                  </Button>
                </>
              ) : null}
            </div>
          </CardContent>
        </Card>
      )}
    </LoginDeviceShell>
  );
}

function LoginDeviceShell({
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

function ApprovalCaution() {
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

function PreviewPanel({
  preview,
  remainingSeconds,
}: {
  readonly preview: PreviewAuthDeviceResponse;
  readonly remainingSeconds: number | null;
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

function TerminalPanel({
  decision,
}: {
  readonly decision: "approved" | "denied";
}) {
  const approved = decision === "approved";
  return (
    <Card
      className={
        approved
          ? "border-success/25 bg-success/[0.03]"
          : "border-destructive/25 bg-destructive/[0.03]"
      }
    >
      <CardContent className="flex flex-col items-center gap-4 p-5 text-center">
        <div
          className={
            approved
              ? "flex h-11 w-11 items-center justify-center rounded-xl border border-success/30 bg-success/10"
              : "flex h-11 w-11 items-center justify-center rounded-xl border border-destructive/30 bg-destructive/10"
          }
        >
          {approved ? (
            <CheckCircle2 className="h-5 w-5 text-success" />
          ) : (
            <CircleX className="h-5 w-5 text-destructive" />
          )}
        </div>
        <div className="space-y-1">
          <h2 className="text-[15px] font-semibold text-foreground">
            {approved ? "Signed in" : "Request denied"}
          </h2>
          <p className="text-[12px] text-muted-foreground">
            {approved
              ? "Return to your terminal. The CLI should finish automatically."
              : "The requesting device cannot complete this login."}
          </p>
        </div>
        {approved ? (
          <Button asChild variant="outline">
            <Link to="/settings" search={{ tab: "sessions" }}>
              Manage sessions
            </Link>
          </Button>
        ) : (
          <Button asChild variant="outline">
            <Link to="/dashboard">Back to dashboard</Link>
          </Button>
        )}
      </CardContent>
    </Card>
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
