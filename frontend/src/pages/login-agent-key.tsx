import { useCallback, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { DeviceApproval } from "@/components/auth/device-approval";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorBanner } from "@/components/shared/error-banner";
import { AgentKeyCreateForm } from "@/components/auth/agent-key-create-form";
import {
  AgentKeyPermissions,
  AgentKeyIssuanceNotice,
} from "@/components/auth/agent-key-permissions";
import { LoginDeviceShell } from "@/components/auth/login-request-preview";
import { LoginCodeStatus } from "@/components/auth/login-code-status";
import {
  useAgentKeyLoginOptions,
  type LoginFlow,
} from "@/hooks/use-agent-key-login";
import { useMintLoginCode } from "@/hooks/use-login-code";
import {
  agentKeyErrorMessage,
  newKeySelection,
  type AgentKeyApprove,
  type AgentKeySummary,
} from "@/schemas/agent-key-login";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import type { LoginCode } from "@/schemas/login-code";
import { draftSummary } from "@/lib/login-permissions";
import { useAuthStore } from "@/stores/auth-store";

function MintLoginCodePage() {
  const { user, isAuthenticated, logout } = useAuthStore();
  const options = useAgentKeyLoginOptions("agent-key", true);
  const mint = useMintLoginCode();
  const [step, setStep] = useState<
    "review" | "options" | "account" | "confirm"
  >("review");
  const [issued, setIssued] = useState<LoginCode | null>(null);
  const [choice, setChoice] = useState("");
  const [draft, setDraft] = useState<CreateApiKeyFormData>();
  const [selection, setSelection] = useState<AgentKeyApprove["selection"]>();
  const [summary, setSummary] = useState<AgentKeySummary>();
  const [credentialExpiry, setCredentialExpiry] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const action = useRef(false);
  const clearCode = useCallback(
    () => setIssued((v) => (v ? { ...v, code: "" } : v)),
    [],
  );
  async function run(work: () => Promise<void>) {
    if (action.current) return;
    action.current = true;
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (failure) {
      setError(agentKeyErrorMessage(failure));
    } finally {
      action.current = false;
      setBusy(false);
    }
  }
  function reviewNew(data: CreateApiKeyFormData) {
    if (!options.data) return;
    const parsed = newKeySelection(data);
    if (!parsed.success) {
      setError(parsed.error.issues[0]?.message ?? "Check the key details.");
      return;
    }
    setDraft(data);
    setSelection(parsed.data);
    setSummary(
      draftSummary(
        data,
        {
          options: options.data,
          connections: options.data.connections ?? [],
          catalog: [],
        },
        user?.id ?? "",
      ),
    );
    setStep("confirm");
  }
  async function generate() {
    if (!isAuthenticated || (step !== "account" && !selection)) return;
    await run(async () => {
      if (step !== "account" && !selection) return;
      const result = await mint.mutateAsync(
        step === "account"
          ? { auth_kind: "account_session" }
          : {
              auth_kind: "agent_key",
              selection: selection!,
              ...(credentialExpiry
                ? {
                    credential_expires_at: new Date(
                      credentialExpiry,
                    ).toISOString(),
                  }
                : {}),
            },
      );
      setIssued(result);
      mint.reset();
    });
  }
  return (
    <LoginDeviceShell>
      <header className="space-y-3 text-center">
        <NyxidIcon className="mx-auto size-10" />
        <h1 className="text-[22px] font-bold">One-time login code</h1>
      </header>
      {error && <ErrorBanner message={error} />}
      {issued ? (
        <LoginCodeStatus
          issued={issued}
          onClearCode={clearCode}
          onNew={() => {
            setIssued(null);
            setStep("review");
            setChoice("");
            setDraft(undefined);
            setSelection(undefined);
            setSummary(undefined);
            setCredentialExpiry("");
          }}
        />
      ) : !isAuthenticated ? (
        <Button asChild>
          <Link to="/login" search={{ return_to: "/login/code" }}>
            Approve on this computer
          </Link>
        </Button>
      ) : (
        <div className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-2 text-[12px]">
            <span>{user?.display_name ?? user?.email}</span>
            <Button
              variant="link"
              disabled={busy}
              onClick={() => void run(logout)}
            >
              Sign out of this browser
            </Button>
          </div>
          {step === "review" && (
            <div className="space-y-3">
              <p className="text-[12px] text-muted-foreground">
                Anyone with this code can redeem the selected access once,
                within five minutes. Restricted Agent Key access is recommended.
              </p>
              <Button
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await options.mutateAsync("");
                    setStep("options");
                  })
                }
              >
                Restricted Agent Key
              </Button>
              <Button disabled={busy} onClick={() => setStep("account")}>
                Full account session
              </Button>
            </div>
          )}
          {step === "account" && (
            <section className="space-y-4">
              <h2 className="text-[15px] font-semibold">
                Confirm full account access
              </h2>
              <p className="text-[12px] text-muted-foreground">
                This code grants a refreshable account session with your
                account, services, credentials, and organization permissions.
              </p>
              <Button disabled={busy} onClick={() => setStep("review")}>
                Back
              </Button>
              <Button
                variant="primary"
                disabled={busy}
                isLoading={busy}
                onClick={() => void generate()}
              >
                Generate account login code
              </Button>
            </section>
          )}
          {step === "options" && options.data && (
            <section className="space-y-4">
              <h2 className="text-[15px] font-semibold">Choose an Agent Key</h2>
              <fieldset disabled={busy} className="space-y-3">
                {options.data.keys.map((key) => (
                  <label
                    key={key.id}
                    className="flex items-start gap-3 rounded-lg border border-border p-3"
                  >
                    <input
                      type="radio"
                      name="mint-key"
                      aria-label={key.name}
                      checked={choice === key.id}
                      onChange={() => setChoice(key.id)}
                      className="mt-1 accent-primary"
                    />
                    <div className="min-w-0 flex-1">
                      <AgentKeyPermissions apiKey={key} />
                    </div>
                  </label>
                ))}
                <label className="flex items-center gap-2 text-[12px]">
                  <input
                    type="radio"
                    name="mint-key"
                    checked={choice === "new"}
                    onChange={() => setChoice("new")}
                    className="accent-primary"
                  />
                  Create a new key
                </label>
              </fieldset>
              {choice === "new" ? (
                <AgentKeyCreateForm
                  options={options.data}
                  label="CLI Agent"
                  initialValues={draft}
                  disabled={busy}
                  onReview={reviewNew}
                />
              ) : (
                <Button
                  disabled={!choice || busy}
                  onClick={() => {
                    const key = options.data?.keys.find((k) => k.id === choice);
                    if (!key) return;
                    setSelection({ kind: "existing", api_key_id: key.id });
                    setSummary(key);
                    setStep("confirm");
                  }}
                >
                  Review permissions
                </Button>
              )}
            </section>
          )}
          {step === "confirm" && summary && (
            <section className="space-y-4">
              <h2 className="text-[15px] font-semibold">
                Confirm effective permissions
              </h2>
              <AgentKeyPermissions apiKey={summary} />
              <AgentKeyIssuanceNotice
                existing={selection?.kind === "existing"}
              />
              <label className="block space-y-2 text-[12px]">
                Login credential expiry (optional)
                <Input
                  type="datetime-local"
                  value={credentialExpiry}
                  disabled={busy}
                  onChange={(e) => setCredentialExpiry(e.target.value)}
                />
              </label>
              <div className="flex flex-wrap justify-end gap-2">
                <Button disabled={busy} onClick={() => setStep("options")}>
                  Back
                </Button>
                <Button disabled={busy} onClick={() => setStep("review")}>
                  Cancel
                </Button>
                <Button
                  variant="primary"
                  isLoading={busy}
                  disabled={
                    busy ||
                    (!!credentialExpiry &&
                      (!Number.isFinite(Date.parse(credentialExpiry)) ||
                        Date.parse(credentialExpiry) <= Date.now() ||
                        (!!summary.expires_at &&
                          Date.parse(credentialExpiry) >
                            Date.parse(summary.expires_at))))
                  }
                  onClick={() => void generate()}
                >
                  Generate restricted login code
                </Button>
              </div>
            </section>
          )}
        </div>
      )}
    </LoginDeviceShell>
  );
}

export function LoginAgentKeyPage({
  flow = "agent-key",
  mint = false,
}: { flow?: LoginFlow; mint?: boolean } = {}) {
  return mint ? <MintLoginCodePage /> : <DeviceApproval flow={flow} />;
}
