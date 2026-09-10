import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  ArrowLeft,
  Monitor,
  ShieldCheck,
  ShieldX,
  Smartphone,
} from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { LoginRequestCodeForm } from "@/components/auth/login-request-code-form";
import { LoginApprovalTerminal } from "@/components/auth/login-approval-terminal";
import { PhoneApprovalPanel } from "@/components/auth/phone-approval-panel";
import { AgentKeySelection } from "@/components/auth/agent-key-selection";
import { FullAccountConfirm } from "@/components/auth/full-account-confirm";
import {
  usePreviewAgentKeyLogin,
  useAgentKeyLoginOptions,
  useApproveAgentKeyLogin,
  useDenyAgentKeyLogin,
  previewAgentKey,
} from "@/hooks/use-agent-key-login";
import { useApproveAuthDevice } from "@/hooks/use-auth-device";
import {
  agentKeyErrorMessage,
  type AgentKeyPreview,
  type AgentKeyApprove,
} from "@/schemas/agent-key-login";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import { userCodeSchema } from "@/schemas/auth-device";
import {
  resolveAuthDeviceDeadlineMs,
  secondsUntilAuthDeviceDeadline,
} from "@/lib/auth-device-time";
import { useAuthStore } from "@/stores/auth-store";
import {
  ApprovalCaution,
  LoginDeviceShell,
  PreviewPanel,
} from "@/components/auth/login-request-preview";

const phonePreview = (code: string) => previewAgentKey(code, "device");

type Step =
  | "enter-code"
  | "review"
  | "phone"
  | "options"
  | "confirm"
  | "account"
  | "terminal";
type Terminal = "approved" | "denied" | "expired";

export function LoginDevicePage() {
  const { user, isAuthenticated } = useAuthStore();
  const [step, setStep] = useState<Step>("enter-code");
  const [code, setCode] = useState("");
  const [context, setContext] = useState<AgentKeyPreview | null>(null);
  const [deadline, setDeadline] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now);
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [newKeyDraft, setNewKeyDraft] = useState<CreateApiKeyFormData>();
  const lastAction = useRef(0);
  const preview = usePreviewAgentKeyLogin("device");
  const options = useAgentKeyLoginOptions("device");
  const approve = useApproveAgentKeyLogin("device");
  const deny = useDenyAgentKeyLogin("device");
  const accountApprove = useApproveAuthDevice();
  const pending =
    preview.isPending ||
    options.isPending ||
    approve.isPending ||
    deny.isPending ||
    accountApprove.isPending;
  const normalized = userCodeSchema.safeParse(code);
  const supportsRestricted = normalized.success && normalized.data.length === 9;
  const remaining =
    deadline === null ? null : secondsUntilAuthDeviceDeadline(deadline, now);
  const expired = remaining === 0;
  const resetPreview = preview.reset,
    resetOptions = options.reset,
    resetApprove = approve.reset,
    resetDeny = deny.reset;

  const finish = useCallback(
    (result: Terminal) => {
      setTerminal(result);
      setStep("terminal");
      setCode("");
      setContext(null);
      setDeadline(null);
      setNewKeyDraft(undefined);
      setError(null);
      resetPreview();
      resetOptions();
      resetApprove();
      resetDeny();
    },
    [resetPreview, resetOptions, resetApprove, resetDeny],
  );

  useEffect(() => {
    if (deadline === null) return;
    const timer = setInterval(() => {
      const now = Date.now();
      setNow(now);
      if (now >= deadline && !pending) finish("expired");
    }, 1000);
    return () => clearInterval(timer);
  }, [deadline, pending, finish]);

  function throttled() {
    if (pending || Date.now() - lastAction.current < 750) return true;
    lastAction.current = Date.now();
    return false;
  }
  async function continueCode() {
    if (!normalized.success || throttled()) return;
    setError(null);
    try {
      const result = await preview.mutateAsync(normalized.data);
      if (result.status !== "pending") {
        finish(result.status === "delivered" ? "approved" : result.status);
        return;
      }
      setContext(result);
      setNow(Date.now());
      setDeadline(
        resolveAuthDeviceDeadlineMs(
          result.expires_at,
          result.seconds_remaining,
        ),
      );
      setStep("review");
    } catch (failure) {
      setError(agentKeyErrorMessage(failure));
    }
  }
  async function loadOptions() {
    if (!normalized.success || expired || throttled()) return;
    setError(null);
    try {
      await options.mutateAsync(normalized.data);
      setStep("options");
    } catch (failure) {
      setError(agentKeyErrorMessage(failure));
    }
  }
  async function decide(
    accepted: boolean,
    selection?: AgentKeyApprove["selection"],
    credentialExpiry = "",
  ) {
    if (
      !normalized.success ||
      expired ||
      throttled() ||
      (accepted && !selection)
    )
      return;
    setError(null);
    try {
      if (accepted && selection)
        await approve.mutateAsync({
          user_code: normalized.data,
          selection,
          ...(credentialExpiry
            ? {
                credential_expires_at: new Date(credentialExpiry).toISOString(),
              }
            : {}),
        });
      else await deny.mutateAsync(normalized.data);
      finish(accepted ? "approved" : "denied");
    } catch (failure) {
      setError(agentKeyErrorMessage(failure));
    }
  }
  return (
    <LoginDeviceShell>
      <header className="space-y-3 text-center">
        <NyxidIcon className="mx-auto size-10" />
        <h1 className="text-[22px] font-bold sm:text-[28px]">Device login</h1>
      </header>
      {step === "terminal" ? (
        <LoginApprovalTerminal
          terminal={terminal}
          error={error}
          onError={setError}
        />
      ) : (
        <div className="space-y-4">
          {step === "enter-code" && (
            <LoginRequestCodeForm
              code={code}
              pending={pending}
              previewPending={preview.isPending}
              valid={normalized.success}
              onCodeChange={setCode}
              onContinue={() => void continueCode()}
            />
          )}
          {context && (
            <>
              <PreviewPanel preview={context} remainingSeconds={remaining} />
              <p className="text-[12px]">
                <span className="text-muted-foreground">
                  Requested profile:{" "}
                </span>
                {context.requested_profile ?? "Not provided"}
              </p>
              <ApprovalCaution />
            </>
          )}
          {expired && (
            <ErrorBanner message="This request has expired. Start Agent Key login again in your terminal." />
          )}
          {error && <ErrorBanner message={error} />}
          {step === "review" && (
            <div className="flex flex-col gap-2">
              {isAuthenticated && supportsRestricted ? (
                <Button
                  disabled={pending || expired}
                  isLoading={options.isPending}
                  onClick={() => void loadOptions()}
                >
                  <Monitor className="size-3" />
                  Restricted Agent Key
                </Button>
              ) : !isAuthenticated ? (
                <Button asChild>
                  <Link
                    to="/login"
                    search={{
                      return_to: "/login/device",
                    }}
                  >
                    <Monitor className="size-3" />
                    Approve on this computer
                  </Link>
                </Button>
              ) : null}
              {isAuthenticated && (
                <Button
                  disabled={pending || expired}
                  onClick={() => {
                    if (!throttled()) setStep("account");
                  }}
                >
                  <ShieldCheck className="size-3" /> Full account session
                </Button>
              )}
              <Button
                disabled={pending || expired}
                onClick={() => {
                  if (!throttled()) setStep("phone");
                }}
              >
                <Smartphone className="size-3" />
                Approve from your phone
              </Button>
              {isAuthenticated && (
                <Button
                  variant="destructive"
                  disabled={pending || expired}
                  onClick={() => void decide(false)}
                >
                  <ShieldX className="size-3" />
                  Reject
                </Button>
              )}
            </div>
          )}
          {step === "phone" &&
            normalized.success &&
            context &&
            deadline !== null && (
              <PhoneApprovalPanel
                code={normalized.data}
                interval={context.interval}
                deadline={deadline}
                onTerminal={finish}
                path="/login/device"
                preview={phonePreview}
              />
            )}
          {step === "phone" && (
            <Button disabled={pending} onClick={() => setStep("review")}>
              <ArrowLeft className="size-3" />
              Back
            </Button>
          )}
          {step === "account" && (
            <FullAccountConfirm
              pending={pending}
              expired={expired}
              confirmLoading={accountApprove.isPending}
              onBack={() => setStep("review")}
              onReject={() => void decide(false)}
              confirmLabel="Approve full account session"
              onConfirm={() => {
                if (!normalized.success || throttled()) return;
                void accountApprove
                  .mutateAsync(normalized.data)
                  .then(() => finish("approved"))
                  .catch((failure: unknown) =>
                    setError(agentKeyErrorMessage(failure)),
                  );
              }}
            />
          )}
          <AgentKeySelection
            step={step}
            options={options.data}
            label={context?.client_label || "CLI Agent"}
            ownerId={user?.id ?? ""}
            ownerName={user?.display_name ?? user?.email ?? "Personal"}
            newKeyDraft={newKeyDraft}
            onDraftChange={setNewKeyDraft}
            pending={pending}
            expired={expired}
            throttled={throttled}
            onError={setError}
            onStepChange={setStep}
            onReject={() => void decide(false)}
            onConfirm={(selection, expiry) =>
              void decide(true, selection, expiry)
            }
            rejectLoading={deny.isPending}
            confirmLoading={approve.isPending}
            rejectLabel="Reject"
            confirmLabel="Approve"
          />
        </div>
      )}
    </LoginDeviceShell>
  );
}
