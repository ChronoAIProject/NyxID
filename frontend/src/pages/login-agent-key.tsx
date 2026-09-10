import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  ArrowLeft,
  ArrowRight,
  Monitor,
  ShieldCheck,
  ShieldX,
  Smartphone,
} from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorBanner } from "@/components/shared/error-banner";
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
  type LoginFlow,
} from "@/hooks/use-agent-key-login";
import { useApproveAuthDevice } from "@/hooks/use-auth-device";
import {
  agentKeyErrorMessage,
  type AgentKeyPreview,
  type AgentKeyApprove,
} from "@/schemas/agent-key-login";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import { useMintLoginCode } from "@/hooks/use-login-code";
import { LoginCodeStatus } from "@/components/auth/login-code-status";
import type { LoginCode } from "@/schemas/login-code";
import {
  formatAuthDeviceUserCodeInput,
  userCodeSchema,
} from "@/schemas/auth-device";
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

type Step =
  | "enter-code"
  | "review"
  | "phone"
  | "options"
  | "confirm"
  | "account"
  | "terminal";
type Terminal = "approved" | "denied" | "expired";

export function LoginAgentKeyPage({
  flow = "agent-key",
  mint = false,
}: { flow?: LoginFlow; mint?: boolean } = {}) {
  const { user, isAuthenticated } = useAuthStore();
  const [step, setStep] = useState<Step>(mint ? "review" : "enter-code");
  const [issued, setIssued] = useState<LoginCode | null>(null);
  const mintCode = useMintLoginCode();
  const clearIssuedCode = useCallback(
    () => setIssued((value) => (value?.code ? { ...value, code: "" } : value)),
    [],
  );
  const [code, setCode] = useState("");
  const [context, setContext] = useState<AgentKeyPreview | null>(null);
  const [deadline, setDeadline] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now);
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [newKeyDraft, setNewKeyDraft] = useState<CreateApiKeyFormData>();
  const lastAction = useRef(0);
  const preview = usePreviewAgentKeyLogin(flow);
  const phonePreview = useCallback(
    (code: string) => previewAgentKey(code, flow),
    [flow],
  );
  const options = useAgentKeyLoginOptions(flow, mint);
  const approve = useApproveAgentKeyLogin(flow);
  const deny = useDenyAgentKeyLogin(flow);
  const accountApprove = useApproveAuthDevice();
  const pending =
    preview.isPending ||
    options.isPending ||
    approve.isPending ||
    deny.isPending ||
    accountApprove.isPending ||
    mintCode.isPending;
  const normalized = userCodeSchema.safeParse(code);
  const supportsRestricted =
    mint ||
    flow === "agent-key" ||
    (normalized.success && normalized.data.length === 9);
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
    if ((!mint && !normalized.success) || expired || throttled()) return;
    setError(null);
    try {
      await options.mutateAsync(normalized.success ? normalized.data : "");
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
      (!mint && !normalized.success) ||
      expired ||
      throttled() ||
      (accepted && !selection)
    )
      return;
    setError(null);
    try {
      if (mint) {
        if (!accepted) {
          setStep("review");
          return;
        }
        if (!selection) return;
        const result = await mintCode.mutateAsync({
          auth_kind: "agent_key",
          selection,
          ...(credentialExpiry
            ? {
                credential_expires_at: new Date(credentialExpiry).toISOString(),
              }
            : {}),
        });
        setIssued(result);
        mintCode.reset();
        return;
      }
      if (!normalized.success) return;
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
        <h1 className="text-[22px] font-bold sm:text-[28px]">
          {mint
            ? "One-time login code"
            : flow === "device"
              ? "Device login"
              : "Agent Key login"}
        </h1>
      </header>
      {issued ? (
        <LoginCodeStatus
          issued={issued}
          onClearCode={clearIssuedCode}
          onNew={() => {
            setIssued(null);
            setStep("review");
          }}
        />
      ) : step === "terminal" ? (
        <LoginApprovalTerminal
          terminal={terminal}
          error={error}
          onError={setError}
        />
      ) : (
        <div className="space-y-4">
          {step === "enter-code" && (
            <form
              className="space-y-3"
              onSubmit={(event) => {
                event.preventDefault();
                void continueCode();
              }}
            >
              <label
                htmlFor="agent-key-code"
                className="text-[12px] font-medium"
              >
                User code
              </label>
              <Input
                id="agent-key-code"
                data-sensitive
                autoComplete="off"
                value={code}
                maxLength={11}
                placeholder="ABCD-EFGH"
                disabled={pending}
                className="h-12 text-center font-mono text-[22px]"
                onChange={(event) =>
                  setCode(formatAuthDeviceUserCodeInput(event.target.value))
                }
              />
              <div className="flex justify-end">
                <Button
                  variant="primary"
                  type="submit"
                  disabled={!normalized.success || pending}
                  isLoading={preview.isPending}
                >
                  <ArrowRight className="size-3" />
                  Continue
                </Button>
              </div>
            </form>
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
              {mint && (
                <p className="text-[12px] text-muted-foreground">
                  Anyone with this code can redeem the selected access once,
                  within five minutes. Restricted Agent Key access is
                  recommended.
                </p>
              )}
              {isAuthenticated && supportsRestricted ? (
                <Button
                  disabled={pending || expired}
                  isLoading={options.isPending}
                  onClick={() => void loadOptions()}
                >
                  <Monitor className="size-3" />
                  {flow === "device" || mint
                    ? "Restricted Agent Key"
                    : "Approve on this computer"}
                </Button>
              ) : !isAuthenticated ? (
                <Button asChild>
                  <Link
                    to="/login"
                    search={{
                      return_to: mint ? "/login/code" : `/login/${flow}`,
                    }}
                  >
                    <Monitor className="size-3" />
                    Approve on this computer
                  </Link>
                </Button>
              ) : null}
              {(flow === "device" || mint) && isAuthenticated && (
                <Button
                  disabled={pending || expired}
                  onClick={() => {
                    if (!throttled()) setStep("account");
                  }}
                >
                  <ShieldCheck className="size-3" /> Full account session
                </Button>
              )}
              {!mint && (
                <Button
                  disabled={pending || expired}
                  onClick={() => {
                    if (!throttled()) setStep("phone");
                  }}
                >
                  <Smartphone className="size-3" />
                  Approve from your phone
                </Button>
              )}
              {!mint && isAuthenticated && (
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
                path={`/login/${flow}`}
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
              confirmLabel={
                mint
                  ? "Generate account login code"
                  : "Approve full account session"
              }
              onConfirm={() => {
                if (mint) {
                  if (throttled()) return;
                  void mintCode
                    .mutateAsync({ auth_kind: "account_session" })
                    .then((result) => {
                      setIssued(result);
                      mintCode.reset();
                    })
                    .catch((failure: unknown) =>
                      setError(agentKeyErrorMessage(failure)),
                    );
                  return;
                }
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
            rejectLabel={mint ? "Cancel" : "Reject"}
            confirmLabel={mint ? "Generate restricted login code" : "Approve"}
          />
        </div>
      )}
    </LoginDeviceShell>
  );
}
