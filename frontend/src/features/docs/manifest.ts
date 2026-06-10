// Docs site navigation manifest — the publish gate for the public /docs site.
//
// Every page here maps a route slug to a markdown file under `/docs/site/<slug>.md`
// (mirrored to `frontend/public/docs/<slug>.md` by the `docsSync()` Vite plugin and
// fetched at runtime). Files under `/docs` that are NOT referenced here stay private.
//
// Phase 1 ships three surfaces (AI-assisted leads) plus a pinned shared Concepts
// group. Phase 2 adds the CLI Guides + Command reference groups (below); a shared
// Developers group is still to come.

export type DocTabId = "ai" | "web" | "cli";

export interface DocPageRef {
  /** Route slug = path under /docs/site without `.md`, e.g. "ai/guides/mcp-proxy". */
  readonly slug: string;
  readonly title: string;
}

export interface DocGroup {
  readonly group: string;
  readonly pages: readonly DocPageRef[];
}

export interface DocTab {
  readonly id: DocTabId;
  readonly label: string;
  /** One-line blurb for the /docs landing card. */
  readonly blurb: string;
  readonly groups: readonly DocGroup[];
}

// Tab bar order. AI-assisted leads (Phase 1 headline + deepest content).
export const DOCS_TABS: readonly DocTab[] = [
  {
    id: "ai",
    label: "AI-assisted",
    blurb: "Use NyxID through Claude Code, Cursor, Codex, and MCP — agents never see raw keys.",
    groups: [
      {
        group: "Get Started",
        pages: [
          { slug: "ai/getting-started/what-is-ai-access", title: "What is AI-assisted access" },
          { slug: "ai/getting-started/connect-your-agent", title: "Connect your agent" },
          { slug: "ai/getting-started/first-agent-call", title: "Your first agent call" },
        ],
      },
      {
        group: "Guides",
        pages: [
          { slug: "ai/guides/claude-code-cursor-codex", title: "Set up Claude Code, Cursor & Codex" },
          { slug: "ai/guides/mcp-proxy", title: "MCP proxy & tool discovery" },
          { slug: "ai/guides/agent-isolation", title: "Isolate agents with scoped keys" },
          { slug: "ai/guides/wrap-rest-api-as-mcp", title: "Wrap a REST API as MCP tools" },
          { slug: "ai/guides/approvals-for-agents", title: "Approvals for agents" },
          { slug: "ai/guides/llms-txt-playbook", title: "The llms.txt playbook" },
          { slug: "ai/guides/aevatar", title: "Connect aevatar" },
        ],
      },
    ],
  },
  {
    id: "web",
    label: "Web",
    blurb: "Manage services, keys, approvals, and organizations from the dashboard.",
    groups: [
      {
        group: "Get Started",
        pages: [
          { slug: "web/getting-started/sign-up", title: "Sign up & sign in" },
          { slug: "web/getting-started/first-connection", title: "Your first connection" },
        ],
      },
      {
        group: "Guides",
        pages: [
          { slug: "web/guides/manage-keys", title: "Manage keys & credentials" },
          { slug: "web/guides/approvals", title: "Set up approvals" },
          { slug: "web/guides/organizations", title: "Share credentials across an org" },
          { slug: "web/guides/developer-apps", title: "Register a developer app" },
          { slug: "web/guides/channel-bots", title: "Connect a channel bot" },
          { slug: "web/guides/account-security", title: "Account & security" },
        ],
      },
    ],
  },
  {
    id: "cli",
    label: "CLI",
    blurb: "Drive every NyxID operation from the nyxid command line.",
    groups: [
      {
        group: "Get Started",
        pages: [
          { slug: "cli/getting-started/install", title: "Install the CLI" },
          { slug: "cli/getting-started/authenticate", title: "Authenticate" },
          { slug: "cli/getting-started/first-connection", title: "Your first connection" },
        ],
      },
      {
        group: "Guides",
        pages: [
          { slug: "cli/guides/connect-a-service", title: "Connect an AI service" },
          { slug: "cli/guides/credential-node", title: "Set up a credential node" },
          { slug: "cli/guides/ssh-node", title: "Set up an SSH node" },
          { slug: "cli/guides/scoped-agent-keys", title: "Create scoped agent keys" },
          { slug: "cli/guides/organizations", title: "Manage organizations" },
          { slug: "cli/guides/channel-bots", title: "Connect a channel bot" },
        ],
      },
      {
        group: "Command reference",
        pages: [
          { slug: "cli/reference/service", title: "nyxid service" },
          { slug: "cli/reference/api-key", title: "nyxid api-key" },
          { slug: "cli/reference/node", title: "nyxid node" },
          { slug: "cli/reference/ssh", title: "nyxid ssh" },
          { slug: "cli/reference/proxy", title: "nyxid proxy" },
          { slug: "cli/reference/catalog", title: "nyxid catalog" },
          { slug: "cli/reference/org", title: "nyxid org" },
          { slug: "cli/reference/mcp", title: "nyxid mcp" },
          { slug: "cli/reference/others", title: "Other commands" },
        ],
      },
    ],
  },
];

// Surface-agnostic groups, pinned to the bottom of every surface sidebar.
// (Developers — SDK/OAuth/API reference — joins this in Phase 2.)
export const DOCS_SHARED: readonly DocGroup[] = [
  {
    group: "Concepts",
    pages: [
      { slug: "shared/concepts/broker-model", title: "The broker model" },
      { slug: "shared/concepts/endpoints-keys-services", title: "Endpoints, keys & services" },
      { slug: "shared/concepts/the-proxy", title: "The proxy" },
      { slug: "shared/concepts/mcp-proxy", title: "The MCP proxy" },
      { slug: "shared/concepts/credential-nodes", title: "Credential nodes" },
      { slug: "shared/concepts/agent-isolation", title: "Agent isolation" },
      { slug: "shared/concepts/approvals", title: "Approvals" },
      { slug: "shared/concepts/organizations", title: "Organizations" },
      { slug: "shared/concepts/oauth-oidc", title: "OAuth & OIDC identity" },
      { slug: "shared/concepts/encryption", title: "Encryption & key management" },
    ],
  },
];

export function allDocPages(): DocPageRef[] {
  return [
    ...DOCS_TABS.flatMap((t) => t.groups.flatMap((g) => g.pages)),
    ...DOCS_SHARED.flatMap((g) => g.pages),
  ];
}

export function findDocPage(slug: string): DocPageRef | undefined {
  return allDocPages().find((p) => p.slug === slug);
}

export function docTabForSlug(slug: string): DocTabId | "shared" {
  const seg = slug.split("/")[0];
  if (seg === "ai" || seg === "web" || seg === "cli") return seg;
  return "shared";
}

/** The full sidebar for a surface: that tab's groups, plus the pinned shared groups. */
export function sidebarForTab(tabId: DocTabId): { surface: readonly DocGroup[]; shared: readonly DocGroup[] } {
  const tab = DOCS_TABS.find((t) => t.id === tabId);
  return { surface: tab?.groups ?? [], shared: DOCS_SHARED };
}
