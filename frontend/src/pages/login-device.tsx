import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  CheckCircle2,
  CircleX,
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
  userCodeSchema,
} from "@/schemas/auth-device";
import { useAuthStore } from "@/stores/auth-store";

const VALID_CODE_LENGTH = 9;
const CLICK_THROTTLE_MS = 750;

export function LoginDevicePage() {
  const navigate = useNavigate();
  const { isAuthenticated, isLoading } = useAuthStore();
  const [userCode, setUserCode] = useState("");
  const [decision, setDecision] = useState<"approved" | "denied" | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const lastClickAtRef = useRef(0);
  const normalizedCode = useMemo(() => {
    const parsed = userCodeSchema.safeParse(userCode);
    return parsed.success ? parsed.data : null;
  }, [userCode]);
  const preview = usePreviewAuthDevice();
  const approve = useApproveAuthDevice();
  const deny = useDenyAuthDevice();
  const isDecisionPending = approve.isPending || deny.isPending;
  const step: "enter-code" | "review" | "terminal" = decision
    ? "terminal"
    : preview.data
      ? "review"
      : "enter-code";

  useEffect(() => {
    if (isLoading || isAuthenticated) return;
    void navigate({ to: "/login", search: { return_to: "/login/device" } });
  }, [isAuthenticated, isLoading, navigate]);

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
  }

  async function handleContinue() {
    if (!normalizedCode || preview.isPending || withinCooldown()) return;
    setSubmitError(null);
    try {
      await preview.mutateAsync(normalizedCode);
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
                clientLabel={preview.data.client_label}
                clientUserAgent={preview.data.client_user_agent}
                clientIp={preview.data.client_ip}
                initiatedAt={preview.data.initiated_at}
                expiresAt={preview.data.expires_at}
                status={preview.data.status}
              />
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
              ) : step === "review" ? (
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
        Only enter a code you generated yourself. If someone sent you this code,
        cancel and start a fresh login from your own terminal.
      </p>
    </div>
  );
}

function PreviewPanel({
  clientLabel,
  clientUserAgent,
  clientIp,
  initiatedAt,
  expiresAt,
  status,
}: {
  readonly clientLabel: string | null;
  readonly clientUserAgent: string | null;
  readonly clientIp: string | null;
  readonly initiatedAt: string | null;
  readonly expiresAt: string | null;
  readonly status: string;
}) {
  return (
    <div className="rounded-xl border border-border/50 bg-white/[0.02]">
      <div className="border-b border-border/50 px-4 py-2.5">
        <div className="flex items-center justify-between gap-3">
          <p className="text-[13px] font-semibold text-foreground">
            Request details
          </p>
          <span className="rounded-md border border-warning/30 bg-warning/10 px-2 py-0.5 text-[10px] font-medium capitalize text-warning">
            {status}
          </span>
        </div>
      </div>
      <div className="divide-y divide-border/30">
        <div className="px-4 py-3 text-[12px] leading-relaxed text-warning">
          Requested from{" "}
          <span className="font-mono text-foreground">
            {clientIp ?? "an unknown IP"}
          </span>{" "}
          at{" "}
          <span className="font-medium text-foreground">
            {initiatedAt ? formatRequestedAt(initiatedAt) : "an unknown time"}
          </span>
          . Reject this request if you do not recognize it.
        </div>
        <PreviewRow label="Client" value={clientLabel ?? "Unknown device"} />
        <PreviewRow
          label="User agent"
          value={clientUserAgent ?? "Not provided"}
          mono
        />
        <PreviewRow
          label="Expires"
          value={expiresAt ? formatAbsoluteTime(expiresAt) : "Unknown"}
        />
      </div>
    </div>
  );
}

function PreviewRow({
  label,
  value,
  mono = false,
}: {
  readonly label: string;
  readonly value: string;
  readonly mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-4 px-4 py-2.5 text-[12px]">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span
        className={
          mono
            ? "min-w-0 break-words text-right font-mono text-[11px] text-foreground"
            : "min-w-0 break-words text-right font-medium text-foreground"
        }
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

function formatRequestedAt(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "an unknown time";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(timestamp);
}

function formatAbsoluteTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "Unknown";
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(timestamp);
}
