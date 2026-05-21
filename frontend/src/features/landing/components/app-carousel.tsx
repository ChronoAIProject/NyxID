import { memo, useMemo } from "react";
import { useTranslation } from "react-i18next";

function BrowserFrame({
  url,
  children,
}: {
  url: string;
  children: React.ReactNode;
}) {
  return (
    <div className="carousel-card w-[480px] shrink-0 overflow-hidden rounded-2xl border border-border/50 bg-card shadow-2xl shadow-primary/5">
      <div className="flex items-center gap-2 border-b border-border/50 bg-muted px-4 py-2.5">
        <div className="flex gap-1.5">
          <div className="h-2.5 w-2.5 rounded-full bg-red-500/60" />
          <div className="h-2.5 w-2.5 rounded-full bg-yellow-500/60" />
          <div className="h-2.5 w-2.5 rounded-full bg-green-500/60" />
        </div>
        <div className="mx-auto rounded-md bg-card px-3 py-0.5 font-mono text-xs text-text-tertiary">
          {url}
        </div>
      </div>
      <div className="bg-background p-5">{children}</div>
    </div>
  );
}

function MobileFrame({
  title,
  badge,
  children,
}: {
  title: string;
  badge: string;
  children: React.ReactNode;
}) {
  return (
    <div className="carousel-card w-[380px] shrink-0 overflow-hidden rounded-2xl border border-border/50 bg-card shadow-2xl shadow-primary/5">
      <div className="flex items-center justify-between border-b border-border/50 bg-muted px-4 py-2.5">
        <div className="font-mono text-xs text-text-tertiary">{title}</div>
        <div className="flex items-center gap-2">
          <div className="h-2 w-2 rounded-full bg-primary" />
          <span className="font-mono text-xs text-text-tertiary">{badge}</span>
        </div>
      </div>
      <div className="bg-background p-5">{children}</div>
    </div>
  );
}

function DashboardCard() {
  return (
    <BrowserFrame url="app.nyxid.io/dashboard">
      <div className="mb-4 flex items-center justify-between">
        <div className="text-base font-semibold text-foreground">NyxID Dashboard</div>
        <div className="flex items-center gap-2">
          <div className="h-2 w-2 rounded-full bg-success" />
          <span className="font-mono text-xs text-muted-foreground">MFA Active</span>
        </div>
      </div>
      <div className="mb-4 grid grid-cols-4 gap-3">
        {[
          { l: "API Keys", v: "3" },
          { l: "Services", v: "7" },
          { l: "Connections", v: "12" },
          { l: "MFA", v: "On" },
        ].map((m) => (
          <div
            key={m.l}
            className="rounded-lg border border-border/50 bg-card p-3"
          >
            <div className="font-mono text-xs text-text-tertiary">{m.l}</div>
            <div className="mt-1 text-lg font-semibold text-nyx-secondary-400">
              {m.v}
            </div>
          </div>
        ))}
      </div>
      <div className="flex flex-wrap gap-2">
        {["Review Pending", "Audit Log", "Manage Keys", "Settings"].map(
          (a) => (
            <span
              key={a}
              className="rounded-lg bg-primary/10 px-2.5 py-1 font-mono text-xs text-nyx-secondary-400"
            >
              {a}
            </span>
          ),
        )}
      </div>
    </BrowserFrame>
  );
}

function ApprovalCard() {
  return (
    <MobileFrame title="NyxID Mobile" badge="Push Received">
      <div className="mb-3 rounded-lg border border-primary/20 bg-primary/5 p-3">
        <div className="mb-0.5 font-mono text-xs text-nyx-secondary-400">
          APPROVAL REQUEST
        </div>
        <div className="font-semibold text-foreground">
          Production Database Access
        </div>
      </div>
      <div className="mb-3 space-y-2">
        {(
          [
            ["Requested by", "jamie@acme.dev"],
            ["Resource", "db-prod-us-east-1"],
            ["Operation", "READ / WRITE"],
            ["Duration", "2 hours"],
          ] as const
        ).map(([l, v]) => (
          <div key={l} className="flex justify-between">
            <span className="font-mono text-xs text-text-tertiary">{l}</span>
            <span className="font-mono text-xs text-foreground">{v}</span>
          </div>
        ))}
      </div>
      <div className="flex gap-2">
        <button className="flex-1 rounded-lg bg-success/20 py-2.5 font-mono text-sm font-semibold text-success">
          Approve
        </button>
        <button className="flex-1 rounded-lg bg-destructive/20 py-2.5 font-mono text-sm font-semibold text-destructive">
          Deny
        </button>
      </div>
    </MobileFrame>
  );
}

function AuditCard() {
  const events = [
    { t: "14:32", a: "Access Approved", u: "jamie@acme.dev", s: "approved" },
    { t: "14:31", a: "Push Sent", u: "priya@health.org", s: "pending" },
    { t: "14:28", a: "Access Denied", u: "bot-ci-deploy", s: "denied" },
    { t: "14:25", a: "Grant Revoked", u: "marcus@startup.io", s: "revoked" },
    { t: "14:20", a: "Access Approved", u: "jamie@acme.dev", s: "approved" },
  ];
  return (
    <BrowserFrame url="app.nyxid.io/audit">
      <div className="mb-3 flex items-center justify-between">
        <div className="text-base font-semibold text-foreground">Audit Trail</div>
        <span className="rounded bg-primary/10 px-2 py-0.5 font-mono text-xs text-nyx-secondary-400">
          Live
        </span>
      </div>
      <div className="space-y-1.5">
        {events.map((e, i) => (
          <div
            key={i}
            className="flex items-center gap-3 rounded-lg border border-border/50 bg-card px-3 py-2"
          >
            <span className="font-mono text-xs text-text-tertiary">{e.t}</span>
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm text-foreground">{e.a}</div>
              <div className="truncate font-mono text-xs text-muted-foreground">
                {e.u}
              </div>
            </div>
            <span
              className={`rounded-full px-2 py-0.5 font-mono text-xs ${e.s === "approved" ? "bg-success/10 text-success" : e.s === "pending" ? "bg-warning/10 text-warning" : "bg-destructive/10 text-destructive"}`}
            >
              {e.s}
            </span>
          </div>
        ))}
      </div>
    </BrowserFrame>
  );
}

function ConnectionsCard() {
  const providers = [
    { n: "Google OAuth", t: "Identity", s: "Connected" },
    { n: "GitHub OAuth", t: "Identity", s: "Connected" },
    { n: "OpenAI API", t: "LLM", s: "Active" },
    { n: "Anthropic API", t: "LLM", s: "Active" },
    { n: "Slack", t: "OAuth", s: "Connected" },
    { n: "AWS KMS", t: "Vault", s: "Configured" },
  ];
  return (
    <BrowserFrame url="app.nyxid.io/connections">
      <div className="mb-3 text-base font-semibold text-foreground">
        Connected Services
      </div>
      <div className="grid grid-cols-2 gap-2">
        {providers.map((p) => (
          <div
            key={p.n}
            className="flex items-center justify-between rounded-lg border border-border/50 bg-card p-3"
          >
            <div>
              <div className="text-sm text-foreground">{p.n}</div>
              <div className="font-mono text-xs text-text-tertiary">{p.t}</div>
            </div>
            <span className="font-mono text-xs text-success">{p.s}</span>
          </div>
        ))}
      </div>
    </BrowserFrame>
  );
}

function LlmGatewayCard() {
  const models = [
    { p: "OpenAI", m: "gpt-4o", ms: "120ms", ok: true },
    { p: "Anthropic", m: "claude-opus-4-6", ms: "95ms", ok: true },
    { p: "Google AI", m: "gemini-2.5-pro", ms: "140ms", ok: true },
    { p: "DeepSeek", m: "deepseek-r1", ms: "180ms", ok: true },
  ];
  return (
    <BrowserFrame url="app.nyxid.io/gateway">
      <div className="mb-3 flex items-center justify-between">
        <div className="text-base font-semibold text-foreground">LLM Gateway</div>
        <div className="flex items-center gap-2">
          <div className="h-2 w-2 rounded-full bg-success" />
          <span className="font-mono text-xs text-muted-foreground">4 active</span>
        </div>
      </div>
      <div className="space-y-1.5">
        {models.map((m) => (
          <div
            key={m.p}
            className="flex items-center gap-3 rounded-lg border border-border/50 bg-card px-3 py-2"
          >
            <div className="flex-1">
              <div className="text-sm text-foreground">{m.p}</div>
              <div className="font-mono text-xs text-text-tertiary">{m.m}</div>
            </div>
            <span className="font-mono text-xs text-text-tertiary">{m.ms}</span>
            <span className="font-mono text-xs text-success">Ready</span>
          </div>
        ))}
      </div>
    </BrowserFrame>
  );
}

function ApiKeysCard() {
  const keys = [
    {
      n: "prod-backend",
      k: "nyx_pk_3f8a...",
      u: "2 min ago",
      scopes: ["proxy:write", "llm:call"],
    },
    {
      n: "ci-deploy-bot",
      k: "nyx_pk_91cb...",
      u: "14 min ago",
      scopes: ["deploy:trigger"],
    },
    {
      n: "staging-readonly",
      k: "nyx_pk_0e2d...",
      u: "3 days ago",
      scopes: ["proxy:read"],
    },
  ];
  return (
    <BrowserFrame url="app.nyxid.io/keys">
      <div className="mb-3 flex items-center justify-between">
        <div className="text-base font-semibold text-foreground">API Keys</div>
        <span className="rounded-lg bg-primary/10 px-2.5 py-1 font-mono text-xs text-nyx-secondary-400">
          + Create
        </span>
      </div>
      <div className="space-y-2">
        {keys.map((k) => (
          <div
            key={k.n}
            className="rounded-lg border border-border/50 bg-card p-3"
          >
            <div className="mb-1 flex justify-between">
              <span className="text-sm font-medium text-foreground">{k.n}</span>
              <span className="font-mono text-xs text-text-tertiary">{k.u}</span>
            </div>
            <div className="mb-1.5 font-mono text-xs text-muted-foreground">
              {k.k}
            </div>
            <div className="flex gap-1">
              {k.scopes.map((s) => (
                <span
                  key={s}
                  className="rounded bg-primary/10 px-1.5 py-0.5 font-mono text-xs text-nyx-secondary-400"
                >
                  {s}
                </span>
              ))}
            </div>
          </div>
        ))}
      </div>
    </BrowserFrame>
  );
}

function RbacCard() {
  const roles = [
    { n: "admin", m: 3, p: ["*"] },
    { n: "engineer", m: 12, p: ["proxy:read", "proxy:write", "llm:call"] },
    { n: "viewer", m: 8, p: ["proxy:read", "audit:read"] },
  ];
  return (
    <BrowserFrame url="app.nyxid.io/roles">
      <div className="mb-3 flex items-center justify-between">
        <div className="text-base font-semibold text-foreground">
          Roles & Permissions
        </div>
        <span className="rounded-lg bg-primary/10 px-2.5 py-1 font-mono text-xs text-nyx-secondary-400">
          + New Role
        </span>
      </div>
      <div className="space-y-2">
        {roles.map((r) => (
          <div
            key={r.n}
            className="rounded-lg border border-border/50 bg-card p-3"
          >
            <div className="mb-1.5 flex items-center gap-2">
              <span className="font-mono text-sm font-medium text-foreground">
                {r.n}
              </span>
              <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-text-tertiary">
                {r.m}
              </span>
            </div>
            <div className="flex flex-wrap gap-1">
              {r.p.map((p) => (
                <span
                  key={p}
                  className="rounded bg-primary/10 px-1.5 py-0.5 font-mono text-xs text-nyx-secondary-400"
                >
                  {p}
                </span>
              ))}
            </div>
          </div>
        ))}
      </div>
    </BrowserFrame>
  );
}

function ActiveGrantsCard() {
  const grants = [
    {
      r: "db-prod-us-east-1",
      u: "jamie@acme.dev",
      e: "45 min left",
      t: "READ/WRITE",
    },
    {
      r: "k8s-staging",
      u: "jamie@acme.dev",
      e: "1h 20m left",
      t: "DEPLOY",
    },
    {
      r: "openai-api-key",
      u: "priya@health.org",
      e: "23h left",
      t: "LLM CALL",
    },
  ];
  return (
    <MobileFrame title="NyxID Mobile" badge="3 active">
      <div className="mb-3 text-base font-semibold text-foreground">Active Grants</div>
      <div className="space-y-2">
        {grants.map((g) => (
          <div
            key={g.r}
            className="rounded-lg border border-border/50 bg-card p-3"
          >
            <div className="mb-1 flex items-center justify-between">
              <span className="text-sm font-medium text-foreground">{g.r}</span>
              <span className="rounded bg-primary/10 px-1.5 py-0.5 font-mono text-xs text-nyx-secondary-400">
                {g.t}
              </span>
            </div>
            <div className="mb-2 font-mono text-xs text-text-tertiary">{g.u}</div>
            <div className="flex items-center justify-between">
              <span className="font-mono text-xs text-warning">{g.e}</span>
              <button className="rounded bg-destructive/15 px-2.5 py-0.5 font-mono text-xs text-destructive">
                Revoke
              </button>
            </div>
          </div>
        ))}
      </div>
    </MobileFrame>
  );
}

function CarouselSet() {
  return (
    <>
      <DashboardCard />
      <ApprovalCard />
      <LlmGatewayCard />
      <AuditCard />
      <ActiveGrantsCard />
      <ConnectionsCard />
      <ApiKeysCard />
      <RbacCard />
    </>
  );
}

const MemoizedSet = memo(CarouselSet);

export function AppCarousel() {
  const { t } = useTranslation();

  const content = useMemo(
    () => (
      <div className="flex animate-scroll gap-6 py-4">
        <MemoizedSet />
        <MemoizedSet />
      </div>
    ),
    [],
  );

  return (
    <section className="py-24">
      <div className="mx-auto max-w-6xl px-6">
        <h2 className="mb-4 text-center text-3xl font-bold tracking-tight text-foreground md:text-4xl">
          {t("carousel.heading")}
        </h2>
        <p className="mx-auto mb-12 max-w-xl text-center text-muted-foreground">
          {t("carousel.subheading")}
        </p>
      </div>

      <div
        className="relative overflow-hidden"
        style={{ contain: "paint" }}
      >
        <div className="pointer-events-none absolute inset-y-0 left-0 z-10 w-24 bg-gradient-to-r from-landing-bg to-transparent" />
        <div className="pointer-events-none absolute inset-y-0 right-0 z-10 w-24 bg-gradient-to-l from-landing-bg to-transparent" />
        {content}
      </div>
    </section>
  );
}
