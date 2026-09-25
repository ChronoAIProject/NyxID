import {
  useCallback,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
} from "react";
import { useLocation } from "@tanstack/react-router";
import {
  ArrowLeft,
  Check,
  Clock3,
  KeyRound,
  ShieldCheck,
  ShieldX,
} from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  LoginDeviceShell,
  PreviewPanel,
  ApprovalCaution,
} from "./login-request-preview";
import { LoginPermissionPicker } from "./login-permission-picker";
import { LoginKeyDraft } from "./login-key-draft";
import { LoginAccessRows } from "./login-access-rows";
import { AgentKeyPermissions } from "./agent-key-permissions";
import { LoginGrantReview } from "./login-grant-review";
import { useAuthStore } from "@/stores/auth-store";
import {
  previewAgentKey,
  useApproveAgentKeyLogin,
  useDenyAgentKeyLogin,
  type LoginFlow,
} from "@/hooks/use-agent-key-login";
import { fetchLoginInventory } from "@/hooks/use-login-inventory";
import { useApproveAuthDevice } from "@/hooks/use-auth-device";
import {
  agentKeyErrorMessage,
  effectivePermissions,
  newKeySelection,
  type AgentKeyPreview,
  type AgentKeyApprove,
} from "@/schemas/agent-key-login";
import {
  formatAuthDeviceUserCodeInput,
  userCodeSchema,
} from "@/schemas/auth-device";
import {
  parseLoginRequestHints,
  type RequestedPermissions,
} from "@/schemas/login-request";
import {
  compareKey,
  draftSummary,
  effectiveLoginConnections,
  exactConnectionDefaults,
  normalizeRequested,
  permissionOptions,
  requestedProblems,
  type LoginInventory,
} from "@/lib/login-permissions";
import {
  resolveAuthDeviceDeadlineMs,
  secondsUntilAuthDeviceDeadline,
} from "@/lib/auth-device-time";
import { clearLoginResume, resolveLoginResume } from "@/lib/login-resume";
import { useLoginApproval, approvalQuery } from "@/hooks/use-login-approval";
import { LoginIdentityCard } from "./login-identity-card";
import { LoginActions } from "./login-actions";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";

type Terminal = "approved" | "denied" | "expired";

export function DeviceApproval({ flow }: { flow: LoginFlow }) {
  const routedQuery = useLocation({ select: (location) => location.searchStr });
  // Router serialization folds repeated singletons into arrays. Validate the browser URL first.
  const query = window.location.search || routedQuery;
  // A different link is a different review. Responses from the previous mount cannot settle it.
  return <ApprovalRequest key={`${flow}:${query}`} flow={flow} query={query} />;
}

function ApprovalRequest({ flow, query }: { flow: LoginFlow; query: string }) {
  const [resumed] = useState(() => resolveLoginResume(flow, query));
  const [hints] = useState(() => {
    const parsed = parseLoginRequestHints(
      approvalQuery(flow, resumed.query),
      flow,
    );
    if (resumed.error) parsed.errors.push(resumed.error);
    return parsed;
  });
  const browserAuth = useAuthStore();
  const { logout } = browserAuth;
  const [code, setCode] = useState(() =>
    formatAuthDeviceUserCodeInput(hints.user_code ?? ""),
  );
  const approval = useLoginApproval(
    flow,
    userCodeSchema.safeParse(code).success ? userCodeSchema.parse(code) : code,
    query,
  );
  const user = approval.identity?.verified
    ? approval.identity.user
    : browserAuth.user;
  const isAuthenticated =
    !!approval.identity?.verified || browserAuth.isAuthenticated;
  const isLoading = browserAuth.isLoading || approval.restoring;
  const [step, setStep] = useState(1);
  const [detailsOpen, setDetailsOpen] = useState(hints.show_details);
  const [completed, setCompleted] = useState(0);
  const [context, setContext] = useState<AgentKeyPreview | null>(null);
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [deadline, setDeadline] = useState<number | null>(null);
  const [now, setNow] = useState(Date.now);
  const [mode, setMode] = useState(hints.login_type);
  const [requested, setRequested] = useState<RequestedPermissions>(() =>
    normalizeRequested(hints),
  );
  const [inventory, setInventory] = useState<LoginInventory | null>(null);
  const [choice, setChoice] = useState("");
  const [draft, setDraft] = useState<CreateApiKeyFormData>();
  const [credentialExpiry, setCredentialExpiry] = useState("");
  const [approvalIdentity, setApprovalIdentity] = useState(
    isAuthenticated ? user?.id : undefined,
  );
  const currentIdentity = isAuthenticated ? user?.id : undefined;
  if (!isLoading && approvalIdentity !== currentIdentity) {
    setApprovalIdentity(currentIdentity);
    setStep(approval.identity?.verified && context ? 2 : 1);
    setCompleted(approval.identity?.verified && context ? 1 : 0);
    setInventory(null);
    setChoice("");
    setDraft(undefined);
    setCredentialExpiry("");
  }
  const alive = useRef(true),
    ended = useRef(false),
    action = useRef(false),
    generation = useRef(0);
  const approve = useApproveAgentKeyLogin(flow),
    deny = useDenyAgentKeyLogin(flow),
    fullApprove = useApproveAuthDevice();
  const supportsRestricted =
    flow === "agent-key" || context?.supports_grant_choice === true;
  const remaining =
    deadline === null ? null : secondsUntilAuthDeviceDeadline(deadline, now);
  const validCode = userCodeSchema.safeParse(code);
  const blocked =
    busy ||
    !isAuthenticated ||
    !!terminal ||
    !!hints.errors.length ||
    remaining === 0;
  const forgetApproval = approval.forget;
  const finish = useCallback(
    (value: Terminal) => {
      if (!alive.current || ended.current) return;
      ended.current = true;
      clearLoginResume(resumed.token);
      forgetApproval();
      generation.current++;
      setTerminal(value);
      setError(null);
      setInventory(null);
      setDraft(undefined);
      setBusy(false);
    },
    [resumed.token, forgetApproval],
  );
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    if (!context || terminal) return;
    const clock = setInterval(() => {
      setNow(Date.now());
      if (deadline !== null && Date.now() >= deadline && !action.current)
        finish("expired");
    }, 1000);
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      if (cancelled || ended.current) return;
      if (!action.current) {
        const epoch = generation.current;
        try {
          const next = await previewAgentKey(code, flow);
          if (
            !cancelled &&
            !action.current &&
            epoch === generation.current &&
            next.status !== "pending"
          )
            finish(next.status === "delivered" ? "approved" : next.status);
        } catch {
          /* A failed observation cannot replace a human decision or permit an approval. */
        }
      }
      if (!cancelled)
        timer = setTimeout(
          () => void poll(),
          Math.max(5, context?.interval ?? 5) * 1000,
        );
    }
    timer = setTimeout(() => void poll(), Math.max(5, context.interval) * 1000);
    return () => {
      cancelled = true;
      clearInterval(clock);
      clearTimeout(timer);
    };
  }, [code, context, deadline, finish, flow, terminal]);

  async function run(work: () => Promise<void>) {
    if (action.current || ended.current) return;
    if (deadline !== null && Date.now() >= deadline) {
      finish("expired");
      return;
    }
    action.current = true;
    generation.current++;
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (failure) {
      if (alive.current && !ended.current) {
        const errorCode = (failure as { errorCode?: number })?.errorCode;
        if (errorCode === 11201 || errorCode === 11901) finish("expired");
        else if (errorCode === 11204 || errorCode === 11904) finish("denied");
        else if (errorCode === 11205 || errorCode === 11905) finish("approved");
        else setError(agentKeyErrorMessage(failure));
      }
    } finally {
      action.current = false;
      if (alive.current) setBusy(false);
    }
  }
  async function verify() {
    if (!validCode.success || hints.errors.length) return;
    await run(async () => {
      const result = await previewAgentKey(validCode.data, flow);
      if (!alive.current || ended.current) return;
      if (result.status !== "pending") {
        finish(result.status === "delivered" ? "approved" : result.status);
        return;
      }
      if (result.seconds_remaining === 0) {
        finish("expired");
        return;
      }
      setDeadline(
        resolveAuthDeviceDeadlineMs(
          result.expires_at,
          result.seconds_remaining,
        ),
      );
      setCode(validCode.data);
      setContext(result);
      setNow(Date.now());
    });
  }
  const startPreview = useEffectEvent(() => {
    void verify();
  });
  useEffect(() => {
    if (hints.user_code) startPreview();
  }, [hints.user_code]);
  const loadInventory = () =>
    approval.identity?.verified
      ? approval.inventory()
      : fetchLoginInventory(flow, code);
  async function continueAccess() {
    if (blocked || (mode === "agent" && !supportsRestricted)) return;
    await run(async () => {
      if (mode === "agent") {
        const next = await loadInventory();
        if (!alive.current || ended.current || !isCurrentApprover()) return;
        setInventory(next);
      }
      if (!alive.current || ended.current || !isCurrentApprover()) return;
      setCompleted(2);
      setStep(3);
    });
  }
  function isCurrentApprover() {
    if (approval.identity?.verified)
      return (
        approval.current.current?.id === approval.identity.id &&
        approval.current.current?.user?.id === user?.id
      );
    const current = useAuthStore.getState();
    return current.isAuthenticated && current.user?.id === user?.id;
  }
  async function decide(
    selection?: AgentKeyApprove["selection"],
    newDraft?: CreateApiKeyFormData,
  ) {
    if (
      blocked ||
      step !== 3 ||
      !context ||
      (mode === "agent" && !supportsRestricted)
    )
      return;
    await run(async () => {
      const fresh = await previewAgentKey(code, flow);
      if (!alive.current || ended.current || !isCurrentApprover()) return;
      if (fresh.status !== "pending") {
        finish(fresh.status === "delivered" ? "approved" : fresh.status);
        return;
      }
      if (mode === "agent") {
        if (
          !selection ||
          !inventory ||
          (flow === "device" && fresh.supports_grant_choice !== true)
        )
          throw new Error(
            "This request no longer supports restricted approval.",
          );
        const next = await loadInventory();
        if (!alive.current || ended.current || !isCurrentApprover()) return;
        const previousKey = newDraft
          ? draftSummary(newDraft, inventory, user?.id ?? "")
          : inventory.options.keys.find(
              (k) =>
                selection.kind === "existing" && k.id === selection.api_key_id,
            );
        const currentKey = newDraft
          ? draftSummary(newDraft, next, user?.id ?? "")
          : next.options.keys.find(
              (k) =>
                selection.kind === "existing" && k.id === selection.api_key_id,
            );
        const previous =
          previousKey && compareKey(previousKey, requested, inventory);
        const current = currentKey && compareKey(currentKey, requested, next);
        setInventory(next);
        if (
          !current?.matches ||
          JSON.stringify({ key: previousKey, access: previous }) !==
            JSON.stringify({ key: currentKey, access: current })
        )
          throw new Error(
            "The key or connection access changed. Review the updated grant before approving.",
          );
        const grant: AgentKeyApprove = {
          user_code: code,
          selection:
            selection.kind === "existing"
              ? {
                  ...selection,
                  permission_snapshot: previousKey?.permission_snapshot,
                }
              : {
                  ...selection,
                  connection_snapshots: (previousKey
                    ? effectiveLoginConnections(previousKey, inventory)
                    : []
                  ).map((c) => ({
                    service_id: c.id,
                    permission_snapshot: c.permission_snapshot ?? "",
                  })),
                },
          ...(credentialExpiry
            ? {
                credential_expires_at: new Date(credentialExpiry).toISOString(),
              }
            : {}),
        };
        if (approval.identity?.verified) {
          await approval.decide({
            selection: grant.selection,
            credential_expires_at: grant.credential_expires_at,
          });
        } else await approve.mutateAsync(grant);
      } else {
        if (flow !== "device")
          throw new Error("This request only permits an Agent Key.");
        if (approval.identity?.verified) await approval.decide(undefined);
        else await fullApprove.mutateAsync(code);
      }
      finish("approved");
    });
  }
  function beginNew() {
    if (!inventory) return;
    setCredentialExpiry("");
    if (!draft)
      setDraft({
        name: hints.key_name || context?.client_label || "Agent Key",
        scopes: requested.permissions.length
          ? (requested.permissions as CreateApiKeyFormData["scopes"])
          : ["read", "proxy"],
        allowed_service_ids: exactConnectionDefaults(
          requested,
          inventory,
          user?.id ?? "",
        ),
        allowed_node_ids: [],
        allow_all_services: false,
        allow_all_nodes: false,
        expires_at:
          hints.expiry_days === "none"
            ? null
            : new Date(Date.now() + Number(hints.expiry_days) * 86400000)
                .toISOString()
                .slice(0, 10),
        platform: hints.platform,
      });
    setChoice("new");
  }
  const comparisons = inventory
    ? inventory.options.keys
        .map((key) => ({
          key,
          comparison: compareKey(key, requested, inventory),
        }))
        .filter((v) => v.comparison.matches)
        .sort(
          (a, b) =>
            a.comparison.extras.length - b.comparison.extras.length ||
            a.key.name.localeCompare(b.key.name),
        )
    : [];
  const keyChoiceTitle = useRef<HTMLHeadingElement>(null);
  const changeKey = () => {
    setCredentialExpiry("");
    setChoice("");
    requestAnimationFrame(() => keyChoiceTitle.current?.focus());
  };
  const chosen = comparisons.find((c) => c.key.id === choice);
  const denyRequest = () =>
    void run(async () => {
      if (approval.identity?.verified) await approval.decide(undefined, true);
      else await deny.mutateAsync(code);
      finish("denied");
    });
  const problems = inventory ? requestedProblems(requested, inventory) : [];
  const title = ["Verify request", "Choose access", "Scope & approval"];
  return (
    <LoginDeviceShell>
      <div
        className={`overflow-hidden rounded-xl border border-border bg-card ${terminal === "expired" ? "mx-auto w-full max-w-sm" : ""}`}
      >
        <div className="space-y-5 p-5 sm:p-6">
          {terminal !== "expired" && (
            <header className="space-y-3 text-center">
              <div className="mx-auto flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
                <NyxidIcon className="size-9" />
              </div>
              <div className="space-y-1.5">
                <h1 className="text-[22px] font-semibold tracking-tight">
                  Approve {flow === "device" ? "device" : "Agent Key"} login
                </h1>
                {!terminal && (
                  <p className="text-[12px] leading-relaxed text-muted-foreground">
                    {step === 1
                      ? context
                        ? "Match this code with the one on your requesting device."
                        : "Enter the code shown on your device or terminal."
                      : "Choose what the requesting device can access."}
                  </p>
                )}
              </div>
            </header>
          )}
          {terminal === "expired" ? (
            <section
              role="status"
              aria-labelledby="expired-request-title"
              className="space-y-5 text-center"
            >
              <div className="mx-auto flex size-12 items-center justify-center rounded-full border border-border bg-background">
                <Clock3
                  aria-hidden="true"
                  className="size-5 text-muted-foreground"
                />
              </div>
              <div className="space-y-2">
                <h1
                  id="expired-request-title"
                  className="text-[22px] font-semibold tracking-tight"
                >
                  Request expired
                </h1>
                <p className="text-[12px] leading-relaxed text-muted-foreground">
                  Get a new code from the device or terminal you’re signing in
                  to, then enter it here.
                </p>
              </div>
              <Button
                asChild
                variant="primary"
                className="h-9 w-full rounded-md"
              >
                <a href={`/login/${flow}`}>Enter a new code</a>
              </Button>
            </section>
          ) : terminal ? (
            <section role="status" className="space-y-3 py-4 text-center">
              {terminal === "approved" ? (
                <ShieldCheck className="mx-auto size-8 text-success" />
              ) : (
                <ShieldX className="mx-auto size-8 text-destructive" />
              )}
              <h2 className="text-[15px] font-semibold">
                {terminal === "approved"
                  ? "Approved — return to the requesting device"
                  : "Request denied"}
              </h2>
              <p className="text-[12px] text-muted-foreground">
                {terminal === "approved"
                  ? "The requester can now receive the approved credential."
                  : "This request cannot be approved. Start a new login on the requesting device."}
              </p>
              {approval.identity?.verified && (
                <p className="text-[12px] text-muted-foreground">
                  {approval.identity.keep_signed_in
                    ? "You are also signed in to NyxID in this browser."
                    : "Identity verified only for this request. No browser sign-in was created."}
                </p>
              )}
            </section>
          ) : (
            <>
              <nav aria-label="Approval steps" className="py-1">
                <ol className="grid grid-cols-3">
                  {title.map((label, index) => {
                    const current = step === index + 1;
                    const complete = completed > index;
                    const reviewing = current && complete;
                    const latest = completed === index;
                    return (
                      <li key={label} className="relative min-w-0">
                        {index < title.length - 1 && (
                          <span
                            aria-hidden="true"
                            className={`absolute left-[calc(50%+1rem)] right-[calc(-50%+1rem)] top-3.5 h-px ${complete ? "bg-success/60" : "bg-border"}`}
                          />
                        )}
                        <button
                          type="button"
                          disabled={busy || index > completed}
                          aria-current={current ? "step" : undefined}
                          className="group relative flex w-full cursor-pointer flex-col items-center gap-2 rounded-lg px-1 text-center text-[11px] focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-primary disabled:cursor-default"
                          onClick={() => setStep(index + 1)}
                        >
                          <span
                            aria-hidden="true"
                            className={`flex size-7 items-center justify-center rounded-full border text-[12px] font-semibold transition-colors ${complete ? "border-success bg-success text-white" : latest ? (current ? "border-primary bg-primary text-white" : "border-primary bg-card text-foreground") : "border-border bg-card text-muted-foreground"} ${reviewing ? "outline outline-1 outline-offset-4 outline-success" : current ? "outline outline-1 outline-offset-4 outline-primary" : ""}`}
                          >
                            {index + 1}
                          </span>
                          <span
                            className={
                              complete
                                ? "font-semibold text-success"
                                : latest
                                  ? "font-semibold text-primary"
                                  : "text-muted-foreground"
                            }
                          >
                            <span className="sr-only">Step {index + 1}: </span>
                            {label}
                            <span
                              className={`mt-1 block text-[10px] font-normal ${complete ? "text-success" : latest ? "text-primary" : "text-muted-foreground"}`}
                            >
                              {reviewing
                                ? "Reviewing"
                                : complete
                                  ? "Completed"
                                  : latest
                                    ? "In progress"
                                    : "Pending"}
                            </span>
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ol>
              </nav>
              {hints.errors.length > 0 && (
                <div role="alert">
                  <ErrorBanner message={hints.errors.join(" ")} />
                </div>
              )}
              {error && (
                <div role="alert">
                  <ErrorBanner message={error} />
                </div>
              )}
              {step > 1 && context && (
                <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
                  <span>
                    {context.client_label ||
                      context.client_app ||
                      "Requesting device"}{" "}
                    ·{" "}
                    <span className="font-mono">
                      {formatAuthDeviceUserCodeInput(code)}
                    </span>{" "}
                    · {remaining}s
                  </span>
                  <Button
                    size="sm"
                    variant="link"
                    disabled={busy}
                    onClick={() => {
                      setDetailsOpen(true);
                      setStep(1);
                    }}
                  >
                    Review request
                  </Button>
                </div>
              )}
              <section hidden={step !== 1} className="space-y-4">
                <h2 className="sr-only">Verify request</h2>
                {!context ? (
                  <form
                    className="space-y-4"
                    onSubmit={(event) => {
                      event.preventDefault();
                      if (!busy && validCode.success && !hints.errors.length)
                        void verify();
                    }}
                  >
                    <label htmlFor="approval-user-code" className="sr-only">
                      User code
                    </label>
                    <Input
                      id="approval-user-code"
                      value={code}
                      disabled={busy}
                      autoComplete="one-time-code"
                      placeholder="XXXX-XXXX"
                      className="h-16 bg-background text-center font-mono text-[22px] uppercase tracking-widest sm:text-[28px]"
                      onChange={(e) => setCode(e.target.value)}
                    />
                    <Button
                      type="submit"
                      variant="primary"
                      className="h-9 w-full rounded-md"
                      disabled={
                        busy || !validCode.success || !!hints.errors.length
                      }
                      isLoading={busy}
                    >
                      Continue
                    </Button>
                  </form>
                ) : (
                  <>
                    <PreviewPanel
                      preview={context}
                      requestedProfile={context.requested_profile}
                      detailsOpen={detailsOpen}
                      onDetailsOpenChange={setDetailsOpen}
                      userCode={code}
                      remainingSeconds={remaining}
                    />
                    <ApprovalCaution />
                    {isLoading ? (
                      <p className="text-[12px]">Verifying your identity…</p>
                    ) : !isAuthenticated ? (
                      <LoginIdentityCard
                        approval={approval}
                        disabled={busy || !!terminal || remaining === 0}
                        onVerified={() => {
                          setCompleted(1);
                          setStep(2);
                        }}
                      />
                    ) : (
                      <LoginActions onDeny={denyRequest} disabled={busy} paired>
                        <Button
                          variant="primary"
                          disabled={blocked}
                          onClick={() => {
                            setCompleted(Math.max(1, completed));
                            setStep(2);
                          }}
                        >
                          Continue
                        </Button>
                      </LoginActions>
                    )}
                  </>
                )}
              </section>
              <section hidden={step !== 2} className="space-y-4">
                <h2 className="text-[15px] font-semibold">Choose access</h2>
                <p className="text-[12px] text-muted-foreground">
                  Choose the access this device should receive.
                </p>
                <div className="grid gap-3 sm:grid-cols-2">
                  {[
                    ...(flow === "device"
                      ? [
                          {
                            value: "full" as const,
                            title: "Full account access",
                            description:
                              "Your account and organization permissions.",
                            detail: "For a device you trust with your account.",
                            Icon: ShieldCheck,
                          },
                        ]
                      : []),
                    ...(supportsRestricted
                      ? [
                          {
                            value: "agent" as const,
                            title: "Restricted Agent Key",
                            description:
                              "Only the access included in the key you choose.",
                            detail: "Use an existing key or create a new one.",
                            Icon: KeyRound,
                          },
                        ]
                      : []),
                  ].map(({ value, title, description, detail, Icon }) => (
                    <label
                      key={value}
                      className="group relative cursor-pointer"
                    >
                      <input
                        type="radio"
                        name="login-access"
                        value={value}
                        checked={mode === value}
                        disabled={blocked}
                        onChange={() => setMode(value)}
                        className="peer absolute inset-0 z-10 h-full w-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
                      />
                      <span className="flex h-full flex-col gap-3 rounded-xl border border-border bg-background/30 p-4 transition-colors group-hover:bg-white/[0.03] peer-checked:border-primary/70 peer-checked:bg-primary/5 peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-primary peer-disabled:cursor-not-allowed peer-disabled:opacity-60">
                        <span className="flex items-center justify-between">
                          <span
                            className={`flex size-9 items-center justify-center rounded-lg border ${mode === value ? "border-primary/25 bg-primary/10 text-primary" : "border-border bg-muted/40 text-muted-foreground"}`}
                          >
                            <Icon
                              aria-hidden="true"
                              className="size-4"
                              strokeWidth={1.75}
                            />
                          </span>
                          <span
                            aria-hidden="true"
                            className={`flex size-4 items-center justify-center rounded-full border ${mode === value ? "border-primary bg-primary text-primary-foreground" : "border-muted-foreground/40"}`}
                          >
                            {mode === value && (
                              <Check className="size-2.5" strokeWidth={3} />
                            )}
                          </span>
                        </span>
                        <span className="space-y-1">
                          <strong className="block text-[13px] font-semibold">
                            {title}
                          </strong>
                          <span className="block text-[12px] leading-relaxed text-muted-foreground">
                            {description}
                          </span>
                        </span>
                        <span className="mt-auto border-t border-border/50 pt-2 text-[11px] text-muted-foreground">
                          {detail}
                        </span>
                      </span>
                    </label>
                  ))}
                </div>
                {mode === "agent" && !supportsRestricted && (
                  <p role="alert" className="text-[12px] text-warning">
                    This legacy request cannot receive an Agent Key. Start a
                    capable request, or explicitly choose full account access.
                  </p>
                )}
                <LoginActions onDeny={denyRequest} disabled={busy}>
                  <Button
                    variant="primary"
                    disabled={
                      blocked || (mode === "agent" && !supportsRestricted)
                    }
                    isLoading={busy}
                    onClick={() => void continueAccess()}
                  >
                    Continue to approval
                  </Button>
                </LoginActions>
              </section>
              <section hidden={step !== 3} className="space-y-4">
                {((!chosen && choice !== "new") || mode === "full") && (
                  <>
                    <h2 className="text-[15px] font-semibold">
                      Scope &amp; approval
                    </h2>
                    <p className="text-[11px] text-muted-foreground">
                      {mode === "full"
                        ? "Full account access · No scope configuration needed."
                        : "Scope optional · Select permissions to find a matching key."}
                    </p>
                  </>
                )}
                {mode === "full" ? (
                  <>
                    <p className="text-[12px] text-muted-foreground">
                      Approve full account access for the requesting device. Its
                      session can refresh until it expires or you revoke it in
                      Settings.
                    </p>
                    <LoginActions onDeny={denyRequest} disabled={busy}>
                      <Button
                        disabled={blocked}
                        isLoading={busy}
                        className="border-success bg-success text-white hover:border-success hover:bg-success/90"
                        onClick={() => void decide()}
                      >
                        Approve full account access
                      </Button>
                    </LoginActions>
                  </>
                ) : (
                  inventory && (
                    <>
                      {choice !== "new" && !chosen && (
                        <LoginPermissionPicker
                          options={permissionOptions(inventory)}
                          value={requested}
                          initial={hints}
                          disabled={blocked}
                          onChange={(next) => {
                            setRequested(next);
                            setChoice((v) => (v === "new" ? v : ""));
                            setError(null);
                          }}
                        />
                      )}
                      {problems.length > 0 && (
                        <p role="alert" className="text-[12px] text-warning">
                          Unknown permissions or services: {problems.join(", ")}
                          . Remove them or clear the filters.
                        </p>
                      )}
                      {choice !== "new" ? (
                        <>
                          {!chosen && (
                            <>
                              <div className="flex justify-between text-[12px]">
                                <h3
                                  ref={keyChoiceTitle}
                                  tabIndex={-1}
                                  className="font-semibold outline-none"
                                >
                                  Choose an existing Agent Key
                                </h3>
                                <span>
                                  {comparisons.length}{" "}
                                  {comparisons.length === 1
                                    ? "match"
                                    : "matches"}
                                </span>
                              </div>
                              <fieldset
                                disabled={blocked}
                                aria-label="Matching Agent Keys"
                                className="max-h-96 space-y-3 overflow-y-auto"
                              >
                                {comparisons.map(({ key, comparison }) => (
                                  <button
                                    type="button"
                                    key={key.id}
                                    disabled={blocked}
                                    onClick={() => setChoice(key.id)}
                                    aria-label={key.name}
                                    className="w-full space-y-2 rounded-xl border border-border p-3 text-left hover:bg-muted/30 focus-visible:outline-2 focus-visible:outline-primary"
                                  >
                                    <span className="flex items-center gap-2 text-[12px]">
                                      <KeyRound
                                        className="size-3 shrink-0"
                                        aria-hidden="true"
                                      />
                                      <strong className="min-w-0 flex-1 break-words">
                                        {key.name}
                                      </strong>
                                      <span
                                        className={`ml-auto shrink-0 text-[10px] ${comparison.exact ? "text-success" : "text-warning"}`}
                                      >
                                        {comparison.exact
                                          ? "Exact match"
                                          : `Matched + ${comparison.extras.length} extra${comparison.extras.length === 1 ? "" : "s"}`}
                                      </span>
                                    </span>
                                    <span className="block break-words pl-6 text-[11px] text-muted-foreground">
                                      {key.owner_name} ·{" "}
                                      {effectivePermissions(key.scopes)} ·{" "}
                                      {key.allow_all_services
                                        ? "All services"
                                        : `${key.allowed_services.length} ${key.allowed_services.length === 1 ? "service" : "services"}`}
                                    </span>
                                  </button>
                                ))}
                              </fieldset>
                              {!comparisons.length && (
                                <p className="text-[12px] text-muted-foreground">
                                  No matching Agent Key. Create one with the
                                  requested access or edit the filters.
                                </p>
                              )}
                              <LoginActions
                                onDeny={denyRequest}
                                disabled={busy}
                              >
                                <Button
                                  variant="primary"
                                  disabled={blocked || !!problems.length}
                                  onClick={beginNew}
                                >
                                  Create new Agent Key
                                </Button>
                              </LoginActions>
                              {hints.key_source === "new" && (
                                <p className="text-[11px] text-muted-foreground">
                                  The requester suggested creating a new key.
                                  You can also choose a matching existing key.
                                </p>
                              )}
                            </>
                          )}
                          {chosen && (
                            <>
                              <Button
                                variant="ghost"
                                size="sm"
                                disabled={blocked}
                                onClick={changeKey}
                              >
                                <ArrowLeft aria-hidden="true" />
                                Change key
                              </Button>
                              <LoginGrantReview
                                key={chosen.key.id}
                                apiKey={chosen.key}
                                comparison={chosen.comparison}
                                kind="existing"
                                credentialExpiresAt={credentialExpiry}
                                customize={
                                  <>
                                    <AgentKeyPermissions apiKey={chosen.key} />
                                    <label className="block space-y-2 text-[12px]">
                                      Login credential expiry (optional)
                                      <Input
                                        type="datetime-local"
                                        aria-label="Login credential expiry (optional)"
                                        value={credentialExpiry}
                                        disabled={blocked}
                                        onChange={(e) =>
                                          setCredentialExpiry(e.target.value)
                                        }
                                      />
                                    </label>
                                  </>
                                }
                                actions={
                                  <LoginActions
                                    onDeny={denyRequest}
                                    disabled={busy}
                                  >
                                    <Button
                                      className="border-success/30 bg-success/10 text-success hover:bg-success/20"
                                      disabled={
                                        blocked ||
                                        (!!credentialExpiry &&
                                          (!Number.isFinite(
                                            Date.parse(credentialExpiry),
                                          ) ||
                                            Date.parse(credentialExpiry) <=
                                              now ||
                                            (!!chosen.key.expires_at &&
                                              Date.parse(credentialExpiry) >
                                                Date.parse(
                                                  chosen.key.expires_at,
                                                ))))
                                      }
                                      isLoading={busy}
                                      onClick={() =>
                                        void decide({
                                          kind: "existing",
                                          api_key_id: chosen.key.id,
                                        })
                                      }
                                    >
                                      Approve access with this key
                                    </Button>
                                  </LoginActions>
                                }
                              >
                                <LoginAccessRows
                                  apiKey={chosen.key}
                                  comparison={chosen.comparison}
                                  inventory={inventory}
                                  requested={requested}
                                  disabled={blocked}
                                />
                              </LoginGrantReview>
                            </>
                          )}
                        </>
                      ) : (
                        draft && (
                          <>
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={blocked}
                              onClick={changeKey}
                            >
                              <ArrowLeft aria-hidden="true" />
                              Back to matching keys
                            </Button>
                            <LoginKeyDraft
                              onDeny={denyRequest}
                              initial={draft}
                              inventory={inventory}
                              requested={requested}
                              initialRequested={hints}
                              onRequestedChange={setRequested}
                              actor={user?.id ?? ""}
                              disabled={blocked}
                              onDraft={setDraft}
                              onApprove={(data) => {
                                const parsed = newKeySelection(data);
                                if (parsed.success)
                                  void decide(parsed.data, data);
                                else
                                  setError(
                                    parsed.error.issues[0]?.message ??
                                      "Check the key settings.",
                                  );
                              }}
                            />
                          </>
                        )
                      )}
                    </>
                  )
                )}
              </section>
            </>
          )}
        </div>
        {isAuthenticated && !terminal && (
          <footer className="flex flex-wrap items-center justify-center gap-x-3 gap-y-1 border-t border-border/50 px-5 py-3 text-[11px] text-muted-foreground sm:px-6">
            <span>
              Approving as{" "}
              <strong className="text-foreground">
                {user?.display_name || user?.email}
              </strong>
              {user?.display_name && user?.email ? ` · ${user.email}` : ""}
            </span>
            <Button
              variant="link"
              size="sm"
              disabled={busy}
              onClick={() => {
                if (action.current) return;
                action.current = true;
                generation.current++;
                setBusy(true);
                void (async () => {
                  const keep = approval.identity?.keep_signed_in;
                  if (approval.identity) await approval.reset();
                  if (browserAuth.isAuthenticated || keep) await logout();
                })()
                  .then(() => {
                    setStep(1);
                    setCompleted(0);
                    setInventory(null);
                    setChoice("");
                    setDraft(undefined);
                  })
                  .catch(() => setError("Could not sign out. Try again."))
                  .finally(() => {
                    action.current = false;
                    if (alive.current) setBusy(false);
                  });
              }}
            >
              {approval.identity && !approval.identity.keep_signed_in
                ? "Change account"
                : "Sign out of this browser"}
            </Button>
          </footer>
        )}
      </div>
      <p className="text-center text-[11px] text-muted-foreground">
        Device login by NyxID
      </p>
    </LoginDeviceShell>
  );
}
