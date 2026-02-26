import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { usePublicConfig } from "@/hooks/use-public-config";

function CodeBlock({ children, label }: { readonly children: string; readonly label?: string }) {
  return (
    <div className="relative">
      {label && (
        <div className="mb-1 flex items-center gap-2">
          <Badge variant="outline" className="text-[10px]">
            {label}
          </Badge>
        </div>
      )}
      <pre className="rounded-lg border border-border bg-muted px-4 py-3 font-mono text-xs overflow-x-auto leading-relaxed">
        {children}
      </pre>
    </div>
  );
}

function Section({
  title,
  description,
  children,
}: {
  readonly title: string;
  readonly description?: string;
  readonly children: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      <CardContent className="space-y-4">{children}</CardContent>
    </Card>
  );
}

function buildCursorExample(mcpUrl: string): string {
  return JSON.stringify(
    { mcpServers: { nyxid: { url: mcpUrl } } },
    null,
    2,
  );
}

function buildClaudeCodeExample(mcpUrl: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        nyxid: {
          command: "npx",
          args: ["-y", "@anthropic-ai/mcp-proxy", mcpUrl],
          description: "NyxID MCP Proxy",
        },
      },
    },
    null,
    2,
  );
}

function buildCodexExample(mcpUrl: string): string {
  return `[mcp_servers.nyxid]\nurl = "${mcpUrl}"`;
}

export function GuidePage() {
  const { data: config, isLoading } = usePublicConfig();

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  const mcpUrl = config?.mcp_url ?? `${window.location.origin}/mcp`;
  const cursorExample = buildCursorExample(mcpUrl);
  const claudeCodeExample = buildClaudeCodeExample(mcpUrl);
  const codexExample = buildCodexExample(mcpUrl);
  return (
    <div className="space-y-8">
      <div>
        <h2 className="font-display text-3xl md:text-5xl font-normal tracking-tight">Guide</h2>
        <p className="text-muted-foreground">
          Learn how to set up and use NyxID for identity management and MCP proxy access.
        </p>
      </div>

      <Section
        title="Overview"
        description="What NyxID does and how it works"
      >
        <p className="text-sm text-muted-foreground leading-relaxed">
          NyxID is an identity and access management platform that also acts as
          an MCP (Model Context Protocol) proxy. It lets you manage
          authentication for services and expose their API endpoints as MCP
          tools that AI clients like Cursor, Claude Code, and Codex can call directly.
        </p>
        <div className="space-y-2">
          <h4 className="text-sm font-medium">Service Types</h4>
          <div className="grid gap-3 sm:grid-cols-3">
            <div className="rounded-lg border p-3">
              <div className="mb-1 flex items-center gap-2">
                <Badge variant="secondary" className="text-xs">SSO Provider</Badge>
              </div>
              <p className="text-xs text-muted-foreground">
                OIDC-based authentication. Users sign in via the provider's OAuth flow.
                Typically used for identity federation.
              </p>
            </div>
            <div className="rounded-lg border p-3">
              <div className="mb-1 flex items-center gap-2">
                <Badge variant="secondary" className="text-xs">External Service</Badge>
              </div>
              <p className="text-xs text-muted-foreground">
                Users bring their own credentials (API key, bearer token, or basic auth)
                to connect to third-party APIs.
              </p>
            </div>
            <div className="rounded-lg border p-3">
              <div className="mb-1 flex items-center gap-2">
                <Badge variant="secondary" className="text-xs">Internal Service</Badge>
              </div>
              <p className="text-xs text-muted-foreground">
                Admin configures a shared credential. Users just enable access
                without managing their own keys.
              </p>
            </div>
          </div>
        </div>
      </Section>

      <Section
        title="Setting Up Services"
        description="How admins register and configure services"
      >
        <div className="space-y-3">
          <div>
            <h4 className="text-sm font-medium mb-1">SSO Provider</h4>
            <p className="text-sm text-muted-foreground leading-relaxed">
              Select the OIDC auth type when creating a service. Configure the
              redirect URIs for the provider's OAuth flow, then share the client
              ID and secret with the users or downstream applications that need
              to authenticate.
            </p>
          </div>
          <Separator />
          <div>
            <h4 className="text-sm font-medium mb-1">External Service</h4>
            <p className="text-sm text-muted-foreground leading-relaxed">
              Select API Key, Bearer Token, or Basic auth type. Each user who
              connects to this service will provide their own credential. This
              is ideal for services where users have individual accounts (e.g.,
              a SaaS API).
            </p>
          </div>
          <Separator />
          <div>
            <h4 className="text-sm font-medium mb-1">Internal Service</h4>
            <p className="text-sm text-muted-foreground leading-relaxed">
              Admin configures a single shared credential during service setup.
              Users only need to enable access -- NyxID injects the shared
              credential on their behalf when proxying requests.
            </p>
          </div>
          <Separator />
          <div>
            <h4 className="text-sm font-medium mb-1">Adding API Endpoints</h4>
            <p className="text-sm text-muted-foreground leading-relaxed">
              After creating a service, add the API endpoints that should be
              exposed as MCP tools. You can define them manually or auto-discover
              them by providing an OpenAPI specification URL.
            </p>
          </div>
        </div>
      </Section>

      <Section
        title="Connecting to Services"
        description="How users connect their accounts to available services"
      >
        <ol className="list-decimal list-inside space-y-2 text-sm text-muted-foreground leading-relaxed">
          <li>Navigate to the <strong className="text-foreground">Connections</strong> page from the sidebar.</li>
          <li>Browse the list of available services and click <strong className="text-foreground">Connect</strong> on the one you want.</li>
          <li>
            For <strong className="text-foreground">External Services</strong>: enter your API key, bearer token,
            or credentials in the form that appears.
          </li>
          <li>
            For <strong className="text-foreground">Internal Services</strong>: click <strong className="text-foreground">Enable</strong> --
            no credentials needed since the admin has configured a shared one.
          </li>
        </ol>
      </Section>

      <Section
        title="Setting Up MCP Clients"
        description="Connect Cursor, Claude Code, or Codex to services via the MCP proxy"
      >
        <div className="rounded-lg border border-primary/20 bg-primary/5 p-4">
          <p className="text-sm text-muted-foreground leading-relaxed">
            NyxID exposes a single MCP endpoint at{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs">/mcp</code>{" "}
            that serves tools from all your connected services. You only need to
            configure your MCP client once -- it will automatically discover
            tools from every service you have enabled on the Connections page.
            You can also use built-in meta-tools to search for tools and
            discover or connect to new services directly from your MCP client.
          </p>
        </div>

        <div className="space-y-4">
          <div>
            <h4 className="text-sm font-medium mb-2">Cursor</h4>
            <ol className="list-decimal list-inside space-y-1 text-sm text-muted-foreground leading-relaxed mb-3">
              <li>
                Create or edit{" "}
                <code className="rounded bg-muted px-1.5 py-0.5 text-xs">.cursor/mcp.json</code>{" "}
                in your project root.
              </li>
              <li>
                Add the NyxID MCP server configuration below.
              </li>
              <li>Restart Cursor -- it will connect to NyxID's MCP proxy.</li>
              <li>When prompted, authenticate via the browser OAuth flow.</li>
              <li>All tools from your connected services will be available.</li>
            </ol>
            <CodeBlock label=".cursor/mcp.json">{cursorExample}</CodeBlock>
          </div>

          <Separator />

          <div>
            <h4 className="text-sm font-medium mb-2">Claude Code</h4>
            <ol className="list-decimal list-inside space-y-1 text-sm text-muted-foreground leading-relaxed mb-3">
              <li>
                Edit{" "}
                <code className="rounded bg-muted px-1.5 py-0.5 text-xs">~/.claude/settings.json</code>{" "}
                or the project-level{" "}
                <code className="rounded bg-muted px-1.5 py-0.5 text-xs">.claude/settings.json</code>.
              </li>
              <li>
                Add the NyxID MCP server configuration below.
              </li>
              <li>Restart Claude Code.</li>
              <li>Authenticate when prompted.</li>
              <li>All tools from your connected services will be available.</li>
            </ol>
            <CodeBlock label=".claude/settings.json">{claudeCodeExample}</CodeBlock>
          </div>

          <Separator />

          <div>
            <h4 className="text-sm font-medium mb-2">Codex</h4>
            <ol className="list-decimal list-inside space-y-1 text-sm text-muted-foreground leading-relaxed mb-3">
              <li>
                Edit{" "}
                <code className="rounded bg-muted px-1.5 py-0.5 text-xs">~/.codex/config.toml</code>.
              </li>
              <li>
                Add the NyxID MCP server configuration below.
              </li>
              <li>Restart Codex.</li>
              <li>Authenticate when prompted.</li>
              <li>All tools from your connected services will be available.</li>
            </ol>
            <CodeBlock label="~/.codex/config.toml">{codexExample}</CodeBlock>
          </div>
        </div>
      </Section>

      <Section
        title="How the MCP Proxy Works"
        description="What happens behind the scenes when an MCP client makes a tool call"
      >
        <ol className="list-decimal list-inside space-y-2 text-sm text-muted-foreground leading-relaxed">
          <li>
            <strong className="text-foreground">Authentication</strong> --
            NyxID authenticates the MCP client via an OAuth flow in the browser.
          </li>
          <li>
            <strong className="text-foreground">Tool Discovery</strong> --
            The proxy aggregates API endpoints from all your connected services
            into a single tool list. Tools are named{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs">service-slug__endpoint-name</code>.
            Built-in meta-tools let you search tools, discover new services,
            and connect to them without leaving your MCP client.
          </li>
          <li>
            <strong className="text-foreground">Credential Injection</strong> --
            When a tool is called, NyxID looks up the user's stored credential
            and injects it into the outgoing request to the downstream API.
          </li>
          <li>
            <strong className="text-foreground">Proxying</strong> --
            The request is forwarded to the downstream service and the response
            is returned to the MCP client.
          </li>
        </ol>
        <div className="mt-2 space-y-2">
          <div className="flex items-start gap-2 rounded-lg border p-3">
            <Badge variant="secondary" className="mt-0.5 shrink-0 text-xs">External</Badge>
            <p className="text-xs text-muted-foreground">
              Uses the individual user's stored credential (API key, bearer token, or basic auth).
            </p>
          </div>
          <div className="flex items-start gap-2 rounded-lg border p-3">
            <Badge variant="secondary" className="mt-0.5 shrink-0 text-xs">Internal</Badge>
            <p className="text-xs text-muted-foreground">
              Uses the admin-configured shared credential. Users do not need
              to manage or even know the underlying credentials.
            </p>
          </div>
        </div>
      </Section>
    </div>
  );
}
