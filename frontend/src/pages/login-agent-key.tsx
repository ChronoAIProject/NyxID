import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import QRCode from "qrcode";
import {
  ArrowLeft,
  ArrowRight,
  CheckCircle2,
  KeyRound,
  LogOut,
  Monitor,
  Plus,
  ShieldCheck,
  ShieldX,
  Smartphone,
} from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorBanner } from "@/components/shared/error-banner";
import { AgentKeyCreateForm } from "@/components/auth/agent-key-create-form";
import {
  AgentKeyPermissions,
  AgentKeyIssuanceNotice,
} from "@/components/auth/agent-key-permissions";
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
  newKeySelection,
  type AgentKeyPreview,
  type AgentKeyApprove,
  type AgentKeySummary,
} from "@/schemas/agent-key-login";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import { useMintLoginCode } from "@/hooks/use-login-code";
import { LoginCodeStatus } from "@/components/auth/login-code-status";
import type { LoginCode } from "@/schemas/login-code";
import {
  formatAuthDeviceUserCodeInput,
  authDeviceUserCodePlaceholder,
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

function localDateTime(value: string): string {
  const date = new Date(value);
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 16);
}

function PhoneApproval({
  code,
  interval,
  deadline,
  onTerminal,
  flow,
}: {
  code: string;
  interval: number;
  deadline: number;
  onTerminal: (state: Terminal) => void;
  flow: LoginFlow;
}) {
  const [qr, setQr] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    const url = new URL(`/login/${flow}`, window.location.origin);
    url.searchParams.set("user_code", code);
    void QRCode.toDataURL(url.toString(), {
      errorCorrectionLevel: "M",
      margin: 4,
      width: 208,
      color: { dark: "#0c0b14", light: "#e8e4f0" },
    })
      .then((image) => {
        if (!cancelled) setQr(image);
      })
      .catch(() => {
        if (!cancelled)
          setError("The QR code could not be displayed. Use the manual code.");
      });
    return () => {
      cancelled = true;
    };
  }, [code, flow]);
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let delay = Math.max(5, interval);
    const poll = async () => {
      if (cancelled) return;
      if (Date.now() >= deadline) {
        onTerminal("expired");
        return;
      }
      try {
        const response = await previewAgentKey(code, flow);
        if (cancelled) return;
        if (response.status === "approved" || response.status === "delivered") {
          onTerminal("approved");
          return;
        }
        if (response.status === "denied" || response.status === "expired") {
          onTerminal(response.status);
          return;
        }
        delay = Math.max(5, response.interval);
        setError(null);
      } catch (pollError) {
        if (cancelled) return;
        const errorCode = (pollError as { errorCode?: number }).errorCode;
        if (errorCode === 11900 || errorCode === 11901 || errorCode === 11907) {
          onTerminal("expired");
          return;
        }
        if (errorCode === 11904) {
          onTerminal("denied");
          return;
        }
        setError(agentKeyErrorMessage(pollError));
        delay = Math.min(30, delay + 5);
      }
      timer = setTimeout(() => void poll(), delay * 1000);
    };
    timer = setTimeout(() => void poll(), delay * 1000);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [code, deadline, interval, onTerminal, flow]);
  return (
    <section className="flex flex-col items-center gap-3 border-t border-border/50 pt-4">
      <h2 className="text-[15px] font-semibold">Approve from your phone</h2>
      <div className="h-52 w-52">
        {qr && (
          <img
            src={qr}
            width={208}
            height={208}
            alt="Agent Key login QR code"
          />
        )}
      </div>
      <p className="font-mono text-[22px]">
        {formatAuthDeviceUserCodeInput(code)}
      </p>
      <p className="text-[12px] text-muted-foreground">
        Waiting for your phone's approval
      </p>
      {error && <ErrorBanner message={error} />}
    </section>
  );
}

export function LoginAgentKeyPage({ flow = "agent-key", mint = false }: { flow?: LoginFlow; mint?: boolean } = {}) {
  const search = useSearch({ strict: false }) as { user_code?: string };
  const navigate = useNavigate();
  useEffect(() => {
    if (!mint && search.user_code !== undefined) {
      void navigate({ to: `/login/${flow}`, search: {}, replace: true });
    }
  }, [flow, mint, navigate, search.user_code]);
  const { user, isAuthenticated, logout } = useAuthStore();
  const [step, setStep] = useState<Step>(mint ? "review" : "enter-code");
  const [issued, setIssued] = useState<LoginCode | null>(null);
  const mintCode = useMintLoginCode();
  const clearIssuedCode = useCallback(() => setIssued((value) => value?.code ? { ...value, code: "" } : value), []);
  const [code, setCode] = useState(() => {
    if (mint || search.user_code === undefined) return "";
    // Validate before the input formatter can strip garbage or truncate a code.
    const parsed = userCodeSchema.safeParse(search.user_code);
    return parsed.success ? formatAuthDeviceUserCodeInput(parsed.data) : "";
  });
  const [context, setContext] = useState<AgentKeyPreview | null>(null);
  const [deadline, setDeadline] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now);
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [error, setError] = useState<string | null>(() =>
    !mint && search.user_code !== undefined && !userCodeSchema.safeParse(search.user_code).success
      ? "The code in this link was not valid. Enter the code manually."
      : null,
  );
  const [choice, setChoice] = useState<string>("");
  const [selection, setSelection] = useState<
    AgentKeyApprove["selection"] | null
  >(null);
  const [summary, setSummary] = useState<AgentKeySummary | null>(null);
  const [credentialExpiry, setCredentialExpiry] = useState("");
  const [newKeyDraft, setNewKeyDraft] = useState<CreateApiKeyFormData>();
  const [signingOut, setSigningOut] = useState(false);
  const lastAction = useRef(0);
  const preview = usePreviewAgentKeyLogin(flow);
  const options = useAgentKeyLoginOptions(flow, mint);
  const approve = useApproveAgentKeyLogin(flow);
  const deny = useDenyAgentKeyLogin(flow);
  const accountApprove = useApproveAuthDevice();
  const pending =
    preview.isPending ||
    options.isPending ||
    approve.isPending ||
    deny.isPending || accountApprove.isPending || mintCode.isPending;
  const normalized = userCodeSchema.safeParse(code);
  const supportsRestricted = mint || flow === "agent-key" || (normalized.success && normalized.data.length === 9);
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
      setSelection(null);
      setSummary(null);
      setChoice("");
      setCredentialExpiry("");
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
  function reviewExisting() {
    if (expired || throttled()) return;
    const key = options.data?.keys.find((key) => key.id === choice);
    if (!key) return;
    setSelection({ kind: "existing", api_key_id: key.id });
    setSummary(key);
    setStep("confirm");
  }
  function reviewNew(data: CreateApiKeyFormData) {
    if (!options.data || expired || throttled()) return;
    const result = newKeySelection(data);
    if (!result.success) {
      setError(result.error.issues[0]?.message ?? "Check the key details.");
      return;
    }
    setError(null);
    const selected = result.data;
    setNewKeyDraft(data);
    const org = options.data.orgs.find(
      (org) => org.id === selected.target_org_id,
    );
    setSelection(selected);
    setSummary({
      id: "",
      name: selected.name,
      key_prefix: "",
      owner_type: org ? "org" : "personal",
      owner_id: org?.id ?? user?.id ?? "",
      owner_name: org?.name ?? user?.display_name ?? user?.email ?? "Personal",
      scopes: selected.scopes,
      allow_all_services: selected.allow_all_services,
      allow_all_nodes: selected.allow_all_nodes,
      allowed_service_ids: selected.allowed_service_ids,
      allowed_node_ids: selected.allowed_node_ids,
      allowed_services: options.data.services.filter((item) =>
        selected.allowed_service_ids.includes(item.id),
      ),
      allowed_nodes: options.data.nodes.filter((item) =>
        selected.allowed_node_ids.includes(item.id),
      ),
      expires_at: selected.expires_at || null,
      rate_limit_per_second: selected.rate_limit_per_second ?? null,
      rate_limit_burst: selected.rate_limit_burst ?? null,
      platform: selected.platform ?? null,
      created_now: true,
    });
    setStep("confirm");
  }
  async function decide(accepted: boolean) {
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
        if (!accepted) { setStep("review"); return; }
        if (!selection) return;
        const result = await mintCode.mutateAsync({ auth_kind: "agent_key", selection,
          ...(credentialExpiry ? { credential_expires_at: new Date(credentialExpiry).toISOString() } : {}),
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
          {mint ? "One-time login code" : flow === "device" ? "Device login" : "Agent Key login"}
        </h1>
      </header>
      {issued ? <LoginCodeStatus issued={issued} onClearCode={clearIssuedCode} onNew={() => {
        setIssued(null); setStep("review"); setSelection(null); setSummary(null); setCredentialExpiry(""); setChoice("");
      }} /> : step === "terminal" ? (
        <section className="space-y-4 py-4 text-center" aria-live="polite">
          {terminal === "approved" ? (
            <CheckCircle2 className="mx-auto size-8 text-success" />
          ) : (
            <ShieldX className="mx-auto size-8 text-destructive" />
          )}
          <h2 className="text-[15px] font-semibold">
            {terminal === "approved"
              ? "Approved - return to the requesting device"
              : terminal === "denied"
                ? "Login rejected"
                : "Login request expired"}
          </h2>
          <Button variant="outline" onClick={() => setStep("enter-code")}>
            Enter another code
          </Button>
          {isAuthenticated && terminal === "approved" && (
            <Button
              variant="outline"
              isLoading={signingOut}
              onClick={() => {
                setSigningOut(true);
                void logout()
                  .catch((failure: unknown) =>
                    setError(agentKeyErrorMessage(failure)),
                  )
                  .finally(() => setSigningOut(false));
              }}
            >
              <LogOut className="size-3" />
              Sign out of this browser
            </Button>
          )}
          {error && <ErrorBanner message={error} />}
        </section>
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
                placeholder={authDeviceUserCodePlaceholder(flow)}
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
              <PreviewPanel preview={context} remainingSeconds={remaining} userCode={code} />
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
              {mint && <p className="text-[12px] text-muted-foreground">Anyone with this code can redeem the selected access once, within five minutes. Restricted Agent Key access is recommended.</p>}
              {isAuthenticated && supportsRestricted ? (
                <Button
                  disabled={pending || expired}
                  isLoading={options.isPending}
                  onClick={() => void loadOptions()}
                >
                  <Monitor className="size-3" />
                  {flow === "device" || mint ? "Restricted Agent Key" : "Approve on this computer"}
                </Button>
              ) : !isAuthenticated ? (
                <Button asChild>
                  <Link to="/login" search={{ return_to: mint ? "/login/code" : `/login/${flow}` }}>
                    <Monitor className="size-3" />
                    Approve on this computer
                  </Link>
                </Button>
              ) : null}
              {(flow === "device" || mint) && isAuthenticated && (
                <Button disabled={pending || expired} onClick={() => {
                  if (!throttled()) setStep("account");
                }}>
                  <ShieldCheck className="size-3" /> Full account session
                </Button>
              )}
              {!mint && <Button
                disabled={pending || expired}
                onClick={() => {
                  if (!throttled()) setStep("phone");
                }}
              >
                <Smartphone className="size-3" />
                Approve from your phone
              </Button>}
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
              <PhoneApproval
                code={normalized.data}
                interval={context.interval}
                deadline={deadline}
                onTerminal={finish}
                flow={flow}
              />
            )}
          {step === "phone" && (
            <Button disabled={pending} onClick={() => setStep("review")}>
              <ArrowLeft className="size-3" />
              Back
            </Button>
          )}
          {step === "account" && (
            <section className="space-y-4 border-t border-border/50 pt-4">
              <h2 className="text-[15px] font-semibold">Confirm full account access</h2>
              <p className="text-[12px] text-muted-foreground">
                This machine receives an account session with access to your account,
                services, credentials, and organization permissions. The session can refresh
                until it expires or you revoke it in Settings.
              </p>
              <div className="flex flex-wrap justify-end gap-2">
                <Button disabled={pending} onClick={() => setStep("review")}><ArrowLeft className="size-3" /> Back</Button>
                <Button variant="destructive" disabled={pending || expired} onClick={() => void decide(false)}><ShieldX className="size-3" /> Reject</Button>
                <Button disabled={pending || expired} isLoading={accountApprove.isPending}
                  className="border-success/30 bg-success/10 text-success hover:bg-success/20"
                  onClick={() => {
                    if (mint) {
                      if (throttled()) return;
                      void mintCode.mutateAsync({ auth_kind: "account_session" }).then((result) => {
                        setIssued(result); mintCode.reset();
                      }).catch((failure: unknown) => setError(agentKeyErrorMessage(failure)));
                      return;
                    }
                    if (!normalized.success || throttled()) return;
                    void accountApprove.mutateAsync(normalized.data).then(() => finish("approved"))
                      .catch((failure: unknown) => setError(agentKeyErrorMessage(failure)));
                  }}><ShieldCheck className="size-3" /> {mint ? "Generate account login code" : "Approve full account session"}</Button>
              </div>
            </section>
          )}
          {step === "options" && options.data && (
            <section className="space-y-4">
              <h2 className="text-[15px] font-semibold">Choose an Agent Key</h2>
              <fieldset disabled={pending || expired} className="space-y-3">
                {options.data.keys.map((key) => (
                  <div
                    key={key.id}
                    className="flex items-start gap-3 rounded-lg border border-border p-3"
                  >
                    <input
                      type="radio"
                      name="agent-key-choice"
                      value={key.id}
                      checked={choice === key.id}
                      onChange={() => setChoice(key.id)}
                      aria-label={key.name}
                      className="mt-1 shrink-0"
                    />
                    <div className="min-w-0 flex-1">
                      <AgentKeyPermissions apiKey={key} />
                    </div>
                  </div>
                ))}
                <label className="flex items-center gap-2 text-[12px]">
                  <input
                    type="radio"
                    name="agent-key-choice"
                    value="new"
                    checked={choice === "new"}
                    onChange={() => setChoice("new")}
                  />
                  <Plus className="size-3" />
                  Create a new key
                </label>
              </fieldset>
              {choice === "new" ? (
                <AgentKeyCreateForm
                  options={options.data}
                  label={context?.client_label || "CLI Agent"}
                  initialValues={newKeyDraft}
                  disabled={pending || expired}
                  onReview={reviewNew}
                />
              ) : (
                <Button
                  variant="primary"
                  disabled={!choice || pending || expired}
                  onClick={reviewExisting}
                >
                  <KeyRound className="size-3" />
                  Review permissions
                </Button>
              )}
            </section>
          )}
          {step === "confirm" && summary && (
            <section className="space-y-4 border-t border-border/50 pt-4">
              <h2 className="text-[15px] font-semibold">
                Confirm effective permissions
              </h2>
              <AgentKeyPermissions apiKey={summary} />
              <AgentKeyIssuanceNotice
                existing={selection?.kind === "existing"}
              />
              <div className="space-y-2">
                <label htmlFor="credential-expiry" className="text-[12px]">
                  Login credential expiry (optional)
                </label>
                <Input
                  id="credential-expiry"
                  type="datetime-local"
                  value={credentialExpiry}
                  disabled={pending || expired}
                  max={
                    summary.expires_at
                      ? localDateTime(summary.expires_at)
                      : undefined
                  }
                  onChange={(event) => setCredentialExpiry(event.target.value)}
                />
              </div>
              <div className="flex flex-wrap justify-end gap-2">
                <Button disabled={pending} onClick={() => setStep("options")}>
                  <ArrowLeft className="size-3" />
                  Back
                </Button>
                <Button
                  variant="destructive"
                  disabled={pending || expired}
                  isLoading={deny.isPending}
                  onClick={() => void decide(false)}
                >
                  <ShieldX className="size-3" />
                  {mint ? "Cancel" : "Reject"}
                </Button>
                <Button
                  className="border-success/30 bg-success/10 text-success hover:bg-success/20"
                  disabled={pending || expired}
                  isLoading={approve.isPending}
                  onClick={() => void decide(true)}
                >
                  <ShieldCheck className="size-3" />
                  {mint ? "Generate restricted login code" : "Approve"}
                </Button>
              </div>
            </section>
          )}
        </div>
      )}
    </LoginDeviceShell>
  );
}
