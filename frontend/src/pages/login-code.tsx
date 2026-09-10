import { useCallback, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Monitor, ShieldCheck } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { AgentKeySelection } from "@/components/auth/agent-key-selection";
import { FullAccountConfirm } from "@/components/auth/full-account-confirm";
import { LoginCodeStatus } from "@/components/auth/login-code-status";
import { LoginDeviceShell } from "@/components/auth/login-request-preview";
import { useLoginCodeOptions, useMintLoginCode } from "@/hooks/use-login-code";
import {
  agentKeyErrorMessage,
  type AgentKeyApprove,
} from "@/schemas/agent-key-login";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import type { LoginCode } from "@/schemas/login-code";
import { useAuthStore } from "@/stores/auth-store";

type Step = "review" | "account" | "options" | "confirm";

export function LoginCodePage() {
  const { user, isAuthenticated } = useAuthStore();
  const [step, setStep] = useState<Step>("review");
  const [issued, setIssued] = useState<LoginCode | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [newKeyDraft, setNewKeyDraft] = useState<CreateApiKeyFormData>();
  const lastAction = useRef(0);
  const options = useLoginCodeOptions();
  const mintCode = useMintLoginCode();
  const pending = options.isPending || mintCode.isPending;
  const clearIssuedCode = useCallback(
    () => setIssued((value) => (value?.code ? { ...value, code: "" } : value)),
    [],
  );

  function throttled() {
    if (pending || Date.now() - lastAction.current < 750) return true;
    lastAction.current = Date.now();
    return false;
  }
  async function loadOptions() {
    if (throttled()) return;
    setError(null);
    try {
      await options.mutateAsync();
      setStep("options");
    } catch (failure) {
      setError(agentKeyErrorMessage(failure));
    }
  }
  function cancel() {
    if (throttled()) return;
    setError(null);
    setStep("review");
  }
  async function generateRestricted(
    selection: AgentKeyApprove["selection"],
    credentialExpiry: string,
  ) {
    if (throttled()) return;
    setError(null);
    try {
      const result = await mintCode.mutateAsync({
        auth_kind: "agent_key",
        selection,
        ...(credentialExpiry
          ? { credential_expires_at: new Date(credentialExpiry).toISOString() }
          : {}),
      });
      setIssued(result);
      mintCode.reset();
    } catch (failure) {
      setError(agentKeyErrorMessage(failure));
    }
  }
  function generateAccount() {
    if (throttled()) return;
    void mintCode
      .mutateAsync({ auth_kind: "account_session" })
      .then((result) => {
        setIssued(result);
        mintCode.reset();
      })
      .catch((failure: unknown) => setError(agentKeyErrorMessage(failure)));
  }

  return (
    <LoginDeviceShell>
      <header className="space-y-3 text-center">
        <NyxidIcon className="mx-auto size-10" />
        <h1 className="text-[22px] font-bold sm:text-[28px]">
          One-time login code
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
      ) : (
        <div className="space-y-4">
          {error && <ErrorBanner message={error} />}
          {step === "review" && (
            <div className="flex flex-col gap-2">
              <p className="text-[12px] text-muted-foreground">
                Anyone with this code can redeem the selected access once,
                within five minutes. Restricted Agent Key access is recommended.
              </p>
              {isAuthenticated ? (
                <Button
                  disabled={pending}
                  isLoading={options.isPending}
                  onClick={() => void loadOptions()}
                >
                  <Monitor className="size-3" />
                  Restricted Agent Key
                </Button>
              ) : (
                <Button asChild>
                  <Link to="/login" search={{ return_to: "/login/code" }}>
                    <Monitor className="size-3" />
                    Approve on this computer
                  </Link>
                </Button>
              )}
              {isAuthenticated && (
                <Button
                  disabled={pending}
                  onClick={() => {
                    if (!throttled()) setStep("account");
                  }}
                >
                  <ShieldCheck className="size-3" /> Full account session
                </Button>
              )}
            </div>
          )}
          {step === "account" && (
            <FullAccountConfirm
              pending={pending}
              expired={false}
              confirmLoading={false}
              onBack={() => setStep("review")}
              onReject={cancel}
              confirmLabel="Generate account login code"
              onConfirm={generateAccount}
            />
          )}
          <AgentKeySelection
            step={step}
            options={options.data}
            label="CLI Agent"
            ownerId={user?.id ?? ""}
            ownerName={user?.display_name ?? user?.email ?? "Personal"}
            newKeyDraft={newKeyDraft}
            onDraftChange={setNewKeyDraft}
            pending={pending}
            expired={false}
            throttled={throttled}
            onError={setError}
            onStepChange={setStep}
            onReject={cancel}
            onConfirm={(selection, expiry) =>
              void generateRestricted(selection, expiry)
            }
            rejectLoading={false}
            confirmLoading={false}
            rejectLabel="Cancel"
            confirmLabel="Generate restricted login code"
          />
        </div>
      )}
    </LoginDeviceShell>
  );
}
