import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useLocation } from "@tanstack/react-router";
import { Check, KeyRound, ShieldCheck, ShieldX } from "lucide-react";
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
import { AccessEntries, LoginKeyDraft } from "./login-key-draft";
import {
  AgentKeyPermissions,
  AgentKeyIssuanceNotice,
} from "./agent-key-permissions";
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
  newKeySelection,
  type AgentKeyPreview,
  type AgentKeyApprove,
} from "@/schemas/agent-key-login";
import {
  formatAuthDeviceUserCodeInput,
  userCodeSchema,
} from "@/schemas/auth-device";
import {
  loginRequestReturnTo,
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
import {
  clearLoginResume,
  resolveLoginResume,
  saveLoginResume,
} from "@/lib/login-resume";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";

type Terminal = "approved" | "denied" | "expired";

export function DeviceApproval({ flow }: { flow: LoginFlow }) {
  const routedQuery = useLocation({ select: (location) => location.searchStr });
  // Router serialization folds repeated singletons into arrays. Validate the browser URL first.
  const query =
    window.location.pathname === `/login/${flow}`
      ? window.location.search
      : routedQuery;
  // A different link is a different review. Responses from the previous mount cannot settle it.
  return <ApprovalRequest key={`${flow}:${query}`} flow={flow} query={query} />;
}

function ApprovalRequest({ flow, query }: { flow: LoginFlow; query: string }) {
  const [resumed] = useState(() => resolveLoginResume(flow, query));
  const [resumeToken] = useState(() => crypto.randomUUID());
  const [hints] = useState(() => {
    const parsed = parseLoginRequestHints(resumed.query, flow);
    if (resumed.error) parsed.errors.push(resumed.error);
    return parsed;
  });
  const { user, isAuthenticated, isLoading, logout } = useAuthStore();
  const [code, setCode] = useState(() =>
    formatAuthDeviceUserCodeInput(hints.user_code ?? ""),
  );
  const [step, setStep] = useState(1);
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
  const finish = useCallback(
    (value: Terminal) => {
      if (!alive.current || ended.current) return;
      ended.current = true;
      clearLoginResume(resumed.token);
      generation.current++;
      setTerminal(value);
      setError(null);
      setInventory(null);
      setDraft(undefined);
      setBusy(false);
    },
    [resumed.token],
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
  async function continueAccess() {
    if (blocked || (mode === "agent" && !supportsRestricted)) return;
    await run(async () => {
      if (mode === "agent") {
        const next = await fetchLoginInventory(flow, code);
        if (!alive.current || ended.current) return;
        setInventory(next);
      }
      if (!alive.current || ended.current) return;
      setCompleted(2);
      setStep(3);
    });
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
      if (!alive.current || ended.current) return;
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
        const next = await fetchLoginInventory(flow, code);
        if (!alive.current || ended.current) return;
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
        await approve.mutateAsync({
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
        });
      } else {
        if (flow !== "device")
          throw new Error("This request only permits an Agent Key.");
        await fullApprove.mutateAsync(code);
      }
      finish("approved");
    });
  }
  function beginNew() {
    if (!inventory) return;
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
  const chosen = comparisons.find((c) => c.key.id === choice);
  const problems = inventory ? requestedProblems(requested, inventory) : [];
  const identityReturn = validCode.success
    ? new URL(
        loginRequestReturnTo(flow, hints, validCode.data),
        window.location.origin,
      ).href
    : "";
  const needsResume = encodeURIComponent(identityReturn).length > 2500;
  const identityTarget = needsResume
    ? new URL(`/login/${flow}?resume=${resumeToken}`, window.location.origin)
        .href
    : identityReturn;
  const title = ["Verify request", "Choose access", "Scope & approval"];
  return (
    <LoginDeviceShell>
      <header className="flex items-center justify-center gap-3">
        <NyxidIcon className="size-7" />
        <h1 className="text-[22px] font-semibold tracking-tight">
          Approve {flow === "device" ? "device" : "Agent Key"} login
        </h1>
      </header>
      {isAuthenticated && (
        <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
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
              void logout()
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
            Sign out of this browser
          </Button>
        </div>
      )}
      {terminal ? (
        <section
          role="status"
          className="space-y-3 rounded-xl border border-border bg-card p-5 text-center"
        >
          {terminal === "approved" ? (
            <ShieldCheck className="mx-auto size-8 text-success" />
          ) : (
            <ShieldX className="mx-auto size-8 text-destructive" />
          )}
          <h2 className="text-[15px] font-semibold">
            {terminal === "approved"
              ? "Approved — return to the requesting device"
              : terminal === "denied"
                ? "Request denied"
                : "Request expired"}
          </h2>
          <p className="text-[12px] text-muted-foreground">
            {terminal === "approved"
              ? "The requester can now receive the approved credential."
              : "This request cannot be approved. Start a new login on the requesting device."}
          </p>
        </section>
      ) : (
        <>
          <nav aria-label="Approval steps" className="grid grid-cols-3 gap-2">
            {title.map((label, index) => (
              <button
                type="button"
                key={label}
                disabled={busy || index > completed}
                aria-current={step === index + 1 ? "step" : undefined}
                className={`flex items-center gap-2 rounded-lg border p-2 text-left text-[11px] disabled:opacity-40 ${step === index + 1 ? "border-primary/40 bg-primary/5" : "border-border"}`}
                onClick={() => setStep(index + 1)}
              >
                <span className="flex size-5 shrink-0 items-center justify-center rounded-full border border-border">
                  {completed > index ? <Check className="size-3" /> : index + 1}
                </span>
                {label}
              </button>
            ))}
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
                onClick={() => setStep(1)}
              >
                Review request
              </Button>
            </div>
          )}
          <section
            hidden={step !== 1}
            className="space-y-4 rounded-xl border border-border bg-card p-4"
          >
            <h2 className="text-[15px] font-semibold">Verify request</h2>
            {!context ? (
              <>
                <label
                  htmlFor="approval-user-code"
                  className="block text-[12px]"
                >
                  User code
                </label>
                <Input
                  id="approval-user-code"
                  value={code}
                  disabled={busy}
                  autoComplete="one-time-code"
                  placeholder="XXXX-XXXX"
                  className="h-12 text-center font-mono text-[22px] tracking-widest"
                  onChange={(e) => setCode(e.target.value)}
                />
                <Button
                  variant="primary"
                  disabled={busy || !validCode.success || !!hints.errors.length}
                  isLoading={busy}
                  onClick={() => void verify()}
                >
                  Continue
                </Button>
              </>
            ) : (
              <>
                <PreviewPanel
                  preview={context}
                  userCode={code}
                  remainingSeconds={remaining}
                />
                {context.requested_profile && (
                  <p className="text-[12px] text-muted-foreground">
                    Requested profile: {context.requested_profile}
                  </p>
                )}
                <ApprovalCaution />
                {isLoading ? (
                  <p className="text-[12px]">Verifying your identity…</p>
                ) : !isAuthenticated ? (
                  <div className="space-y-2">
                    <p className="text-[12px] text-muted-foreground">
                      Verify your identity to review access. Signing in does not
                      approve this request.
                    </p>
                    <Link
                      to="/login"
                      search={{
                        return_to: identityTarget,
                      }}
                      className="text-[12px] text-primary underline"
                      onClick={(event) => {
                        if (!needsResume) return;
                        try {
                          saveLoginResume(
                            resumeToken,
                            flow,
                            new URL(identityReturn).search,
                          );
                        } catch {
                          event.preventDefault();
                          setError(
                            "This browser could not save the request for sign-in. Enable session storage or use a shorter request link.",
                          );
                        }
                      }}
                    >
                      Verify identity to continue
                    </Link>
                  </div>
                ) : (
                  <Button
                    variant="primary"
                    disabled={blocked}
                    onClick={() => {
                      setCompleted(Math.max(1, completed));
                      setStep(2);
                    }}
                  >
                    This is my request — continue
                  </Button>
                )}
              </>
            )}
          </section>
          <section
            hidden={step !== 2}
            className="space-y-4 rounded-xl border border-border bg-card p-4"
          >
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
                <label key={value} className="group relative cursor-pointer">
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
                This legacy request cannot receive an Agent Key. Start a capable
                request, or explicitly choose full account access.
              </p>
            )}
            <Button
              variant="primary"
              disabled={blocked || (mode === "agent" && !supportsRestricted)}
              isLoading={busy}
              onClick={() => void continueAccess()}
            >
              Continue to approval
            </Button>
          </section>
          <section
            hidden={step !== 3}
            className="space-y-4 rounded-xl border border-border bg-card p-4"
          >
            <h2 className="text-[15px] font-semibold">Scope &amp; approval</h2>
            <p className="text-[11px] text-muted-foreground">
              {mode === "full"
                ? "Full account access · No scope configuration needed."
                : choice === "new"
                  ? "Scope optional · Configure the Agent Key to create."
                  : "Scope optional · Select permissions to find a matching key."}
            </p>
            {mode === "full" ? (
              <>
                <p className="text-[12px] text-muted-foreground">
                  Approve full account access for the requesting device. Its
                  session can refresh until it expires or you revoke it in
                  Settings.
                </p>
                <Button
                  disabled={blocked}
                  isLoading={busy}
                  className="border-success/30 bg-success/10 text-success hover:bg-success/20"
                  onClick={() => void decide()}
                >
                  Approve full account access
                </Button>
              </>
            ) : (
              inventory && (
                <>
                  {choice !== "new" && (
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
                      Unknown permissions or services: {problems.join(", ")}.
                      Remove them or clear the filters.
                    </p>
                  )}
                  {choice !== "new" ? (
                    <>
                      <div className="flex justify-between text-[12px]">
                        <h3 className="font-semibold">
                          Choose an existing Agent Key
                        </h3>
                        <span>{comparisons.length} matches</span>
                      </div>
                      <fieldset
                        disabled={blocked}
                        aria-label="Matching Agent Keys"
                        className="max-h-96 space-y-3 overflow-y-auto"
                      >
                        {comparisons.map(({ key, comparison }) => (
                          <div
                            key={key.id}
                            className={`space-y-3 rounded-xl border p-3 ${choice === key.id ? "border-primary/40" : "border-border"}`}
                          >
                            <label className="flex items-center gap-2 text-[12px]">
                              <input
                                className="size-4 shrink-0 appearance-none rounded-full border border-muted-foreground/50 bg-transparent checked:border-primary checked:bg-primary checked:shadow-[inset_0_0_0_3px_var(--color-card)] focus-visible:outline-2 focus-visible:outline-primary"
                                type="radio"
                                name="existing-key"
                                checked={choice === key.id}
                                onChange={() => setChoice(key.id)}
                                aria-label={key.name}
                              />
                              <KeyRound className="size-3" />
                              <strong>{key.name}</strong>
                              <span
                                className={`ml-auto text-[10px] ${comparison.exact ? "text-success" : "text-warning"}`}
                              >
                                {comparison.exact
                                  ? "Exact match"
                                  : `Matched + ${comparison.extras.length} extra${comparison.extras.length === 1 ? "" : "s"}`}
                              </span>
                            </label>
                            <p className="text-[11px] text-muted-foreground">
                              {key.owner_name} ·{" "}
                              {key.owner_type === "org"
                                ? "Organization"
                                : "Personal"}
                            </p>
                            <AccessEntries
                              label="Matched permissions"
                              entries={comparison.matched}
                            />
                            <AccessEntries
                              label="Also grants — included with this key"
                              entries={comparison.extras}
                            />
                            <details className="text-[11px]">
                              <summary className="cursor-pointer text-muted-foreground">
                                All key details
                              </summary>
                              <div className="mt-2">
                                <AgentKeyPermissions apiKey={key} />
                              </div>
                            </details>
                          </div>
                        ))}
                      </fieldset>
                      {!comparisons.length && (
                        <p className="text-[12px] text-muted-foreground">
                          No matching Agent Key. Create one with the requested
                          access or edit the filters.
                        </p>
                      )}
                      <Button
                        disabled={blocked || !!problems.length}
                        onClick={beginNew}
                      >
                        Create new Agent Key
                      </Button>
                      {hints.key_source === "new" && (
                        <p className="text-[11px] text-muted-foreground">
                          The requester suggested creating a new key. You can
                          also choose a matching existing key.
                        </p>
                      )}
                      {chosen && (
                        <>
                          <AgentKeyIssuanceNotice existing />
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
                          <Button
                            className="border-success/30 bg-success/10 text-success hover:bg-success/20"
                            disabled={
                              blocked ||
                              (!!credentialExpiry &&
                                (!Number.isFinite(
                                  Date.parse(credentialExpiry),
                                ) ||
                                  Date.parse(credentialExpiry) <= now ||
                                  (!!chosen.key.expires_at &&
                                    Date.parse(credentialExpiry) >
                                      Date.parse(chosen.key.expires_at))))
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
                        </>
                      )}
                    </>
                  ) : (
                    draft && (
                      <>
                        <Button
                          variant="link"
                          disabled={blocked}
                          onClick={() => setChoice("")}
                        >
                          Back to matching keys
                        </Button>
                        <LoginKeyDraft
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
                            if (parsed.success) void decide(parsed.data, data);
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
          {context && isAuthenticated && (
            <div className="flex justify-end">
              <Button
                variant="destructive"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await deny.mutateAsync(code);
                    finish("denied");
                  })
                }
              >
                Deny request
              </Button>
            </div>
          )}
        </>
      )}
    </LoginDeviceShell>
  );
}
