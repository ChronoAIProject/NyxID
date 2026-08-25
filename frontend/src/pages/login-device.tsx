import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  CircleX,
  Clock3,
  Cpu,
  Globe2,
  Languages,
  MapPin,
  Monitor,
  MonitorSmartphone,
  ShieldCheck,
  ShieldX,
  Timer,
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
            Sign in to NyxID CLI on another device
          </h1>
          <p className="mx-auto max-w-md text-[12px] text-muted-foreground">
            Confirm the one-time code shown by{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
              nyxid login --device
            </code>
            .
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

            <WarningBanner />

            {step === "review" && preview.data ? (
              <PreviewPanel
                preview={preview.data}
                remainingSeconds={previewRemaining}
              />
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

function WarningBanner() {
  return (
    <div className="flex gap-3 rounded-xl border border-warning/20 bg-warning/[0.04] px-4 py-3">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-warning/10">
        <AlertTriangle className="h-4 w-4 text-warning" />
      </div>
      <p className="text-[12px] leading-relaxed text-warning">
        Only approve a request you started yourself. Check the initiating site,
        location, and timing below before deciding. Device names, browser
        details, timezone, and hardware are self-reported and can be forged. If
        someone else showed or sent you this code, reject it.
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
  const location = formatVerifiedLocation(preview);
  const screenDescription = formatScreenDescription(preview);
  const appDescription =
    preview.client_app ??
    (preview.client_kind === "unknown"
      ? "Not identified"
      : `${preview.client_kind[0]?.toUpperCase()}${preview.client_kind.slice(1)} client`);

  return (
    <section className="space-y-5 border-t border-border/50 pt-4" aria-label="Request details">
      <div className="flex items-center justify-between gap-3">
        <p className="text-[13px] font-semibold text-foreground">
          Request details
        </p>
        <span
          className={
            expired
              ? "rounded-md border border-destructive/30 bg-destructive/10 px-2 py-0.5 text-[10px] font-medium text-destructive"
              : "rounded-md border border-warning/30 bg-warning/10 px-2 py-0.5 text-[10px] font-medium capitalize text-warning"
          }
        >
          {expired ? "Request expired" : preview.status}
        </span>
      </div>

      <div>
        <div className="mb-2 flex items-center gap-2 text-[11px] font-semibold uppercase text-success">
          <ShieldCheck className="size-3.5" />
          Verified by NyxID
        </div>
        <div className="divide-y divide-border/30 rounded-md border border-border/50 bg-overlay/30">
          <InitiatingOriginSignal preview={preview} />
          {verifiedIp && preview.client_ip ? (
            <PreviewRow
              icon={<Globe2 />}
              label="Requester IP"
              value={preview.client_ip}
              mono
            />
          ) : (
            <div className="px-3 py-2.5 text-[12px] leading-relaxed text-muted-foreground">
              {preview.client_ip_attribution === "unavailable"
                ? "Requester IP is not available on this deployment."
                : "No requester IP was verified by NyxID."}
            </div>
          )}
          {verifiedIp && location ? (
            <PreviewRow
              icon={<MapPin />}
              label="Requester location"
              value={location}
            />
          ) : null}
          {verifiedIp && networkRelation ? (
            <div
              className={
                networkRelation === "same_ip" || networkRelation === "same_network"
                  ? "flex items-center gap-2 px-3 py-2.5 text-[12px] font-medium text-success"
                  : "flex items-center gap-2 px-3 py-2.5 text-[12px] font-medium text-muted-foreground"
              }
            >
              {networkRelation === "same_ip" ? <ShieldCheck /> : <Globe2 />}
              {networkRelation === "same_ip"
                ? "Same IP as this device"
                : networkRelation === "same_network"
                  ? "Same network as this device"
                  : networkRelation === "different_network"
                    ? "Different network from this device - common when a phone uses cellular data"
                    : "Different IP from this device - common when devices use separate connections"}
            </div>
          ) : null}
          {verifiedIp && preview.client_ip_timezone ? (
            <PreviewRow
              icon={<Globe2 />}
              label="IP timezone"
              value={preview.client_ip_timezone}
            />
          ) : null}
          {preview.client_timezone_matches_ip === false ? (
            <div className="flex items-center gap-2 px-3 py-2.5 text-[12px] font-semibold text-warning">
              <AlertTriangle className="size-3.5 shrink-0" />
              Reported timezone does not match the verified IP timezone
            </div>
          ) : null}
          <PreviewRow
            icon={<Clock3 />}
            label="Requested"
            value={
              <>
                <span className="block">
                  {formatAuthDeviceRelativeTime(preview.initiated_at)}
                </span>
                <span className="mt-0.5 block text-[11px] text-muted-foreground">
                  {formatAbsoluteDateTime(preview.initiated_at)}
                </span>
              </>
            }
          />
          <PreviewRow
            icon={<Timer />}
            label="Expiry"
            value={
              expired
                ? "Expired"
                : `Expires in ${formatWebAuthDeviceRemaining(remainingSeconds ?? 0)}`
            }
            valueClassName={expired ? "text-destructive" : "text-warning"}
          />
        </div>
      </div>

      <div>
        <div className="mb-2 flex items-center gap-2 text-[11px] font-semibold uppercase text-warning">
          <MonitorSmartphone className="size-3.5" />
          Reported by the requesting device (unverified)
        </div>
        <div className="divide-y divide-border/30 rounded-md border border-warning/20 bg-warning/[0.03]">
          {unverifiedIp && preview.client_ip ? (
            <PreviewRow
              icon={<Globe2 />}
              label="Reported IP (unverified)"
              value={preview.client_ip}
              mono
              valueClassName="text-warning"
            />
          ) : null}
          <PreviewRow
            label="Device label"
            value={preview.client_label ?? "Not provided"}
          />
          <PreviewRow label="Client" value={appDescription} />
          <PreviewRow
            label="Platform"
            value={preview.client_platform ?? "Not identified"}
          />
          {preview.client_model ? (
            <PreviewRow label="Device model" value={preview.client_model} />
          ) : null}
          {preview.client_form_factor ? (
            <PreviewRow
              label="Form factor"
              value={capitalize(preview.client_form_factor)}
            />
          ) : null}
          {preview.client_timezone ? (
            <>
              <PreviewRow
                icon={<Globe2 />}
                label="Timezone"
                value={preview.client_timezone}
              />
              {reportedTimezoneDiffers ? (
                <div className="flex items-center gap-2 px-3 py-2.5 text-[12px] font-medium text-warning">
                  <AlertTriangle className="size-3.5 shrink-0" />
                  Differs from this device ({localTimezone})
                </div>
              ) : null}
            </>
          ) : null}
          {preview.client_locale ? (
            <PreviewRow
              icon={<Languages />}
              label="Locale"
              value={preview.client_locale}
            />
          ) : null}
          {screenDescription ? (
            <PreviewRow
              icon={<Monitor />}
              label="Screen"
              value={screenDescription}
            />
          ) : null}
          {preview.client_hardware_concurrency !== null ? (
            <PreviewRow
              icon={<Cpu />}
              label="Processor"
              value={`${preview.client_hardware_concurrency} logical processors`}
            />
          ) : null}
          {preview.client_device_memory !== null ? (
            <PreviewRow
              label="Memory"
              value={`${preview.client_device_memory} GB reported memory`}
            />
          ) : null}
          <details className="group px-3 py-2.5">
            <summary className="flex cursor-pointer list-none items-center justify-between gap-3 text-[12px] text-muted-foreground">
              Raw user agent
              <ChevronDown className="size-3.5 transition-transform group-open:rotate-180" />
            </summary>
            <p className="mt-2 break-all rounded bg-background/60 p-2 font-mono text-[11px] leading-relaxed text-foreground">
              {preview.client_user_agent ?? "Not provided"}
            </p>
          </details>
        </div>
      </div>
    </section>
  );
}

function InitiatingOriginSignal({
  preview,
}: {
  readonly preview: PreviewAuthDeviceResponse;
}) {
  if (preview.initiating_origin_status === "absent") return null;
  const host = originHost(preview.initiating_origin);
  if (preview.initiating_origin_status === "matched") {
    return (
      <div className="flex items-center gap-2 px-3 py-2.5 text-[12px] font-semibold text-success">
        <ShieldCheck className="size-3.5 shrink-0" />
        Started from {host ?? "the configured NyxID site"}
      </div>
    );
  }

  const message =
    preview.initiating_origin_status === "mismatched"
      ? `This sign-in was started from ${host ?? "another site"}, not the official NyxID site. Reject it unless you intentionally used that site.`
      : preview.initiating_origin_status === "non_http"
        ? "This sign-in reported a non-HTTP(S) initiating origin. Reject it unless you generated the request yourself."
        : "The initiating Origin header was malformed. Reject this request unless you generated it yourself.";
  return (
    <div
      role="alert"
      className="flex items-start gap-2 bg-destructive/10 px-3 py-3 text-[12px] font-semibold leading-relaxed text-destructive"
    >
      <ShieldX className="mt-0.5 size-4 shrink-0" />
      {message}
    </div>
  );
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

function formatVerifiedLocation(preview: PreviewAuthDeviceResponse): string | null {
  const locality = [preview.client_city, preview.client_region]
    .filter((value): value is string => Boolean(value))
    .join(", ");
  if (locality && preview.client_country) return `${locality} (${preview.client_country})`;
  return locality || preview.client_country;
}

function formatScreenDescription(preview: PreviewAuthDeviceResponse): string | null {
  if (preview.client_screen_width === null || preview.client_screen_height === null) {
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

function PreviewRow({
  icon,
  label,
  value,
  mono = false,
  valueClassName = "",
}: {
  readonly icon?: React.ReactNode;
  readonly label: string;
  readonly value: React.ReactNode;
  readonly mono?: boolean;
  readonly valueClassName?: string;
}) {
  return (
    <div className="flex items-start justify-between gap-4 px-3 py-2.5 text-[12px]">
      <span className="flex shrink-0 items-center gap-2 text-muted-foreground">
        {icon ? <span className="[&_svg]:size-3.5">{icon}</span> : null}
        {label}
      </span>
      <div
        className={`${
          mono
            ? "min-w-0 break-all text-right font-mono text-[11px] text-foreground"
            : "min-w-0 break-words text-right font-medium text-foreground"
        } ${valueClassName}`}
      >
        {value}
      </div>
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
