import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { Label } from "@/components/ui/label";
import { useState, useMemo, useCallback } from "react";
import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import { toast } from "sonner";
import { Copy, Plus, Shield } from "lucide-react";
import { BrainIcon } from "@/components/icons/empty-state";
import { useDeveloperApps } from "@/hooks/use-developer-apps";
import { usePublicConfig } from "@/hooks/use-public-config";
import {
  AI_TOOLS,
  AI_TOOL_SKILL_INFO,
  generateToolConfig,
  generateSetupPrompt,
  type AiTool,
  type AiToolConfigParams,
} from "@/lib/ai-tool-configs";
import {
  AI_SETUP_SKILL_TABS,
  AI_SETUP_SKILL_TAB_DEFAULT,
  isValidTab,
  parseTab,
} from "@/lib/url-tabs";

function CodeBlock({
  code,
  label,
  onCopy,
}: {
  readonly code: string;
  readonly label: string;
  readonly onCopy: () => void;
}) {
  useUiTranslation();
  return (
    <div className="space-y-2">
      <Badge variant="secondary" className="text-12">
        {label}
      </Badge>
      <div className="relative">
        <pre className="whitespace-pre-wrap break-all rounded-lg border border-border bg-muted px-4 py-3.5 pr-12 min-h-[44px] font-mono text-14 text-foreground">
          {code}
        </pre>
        <Button
          variant="ghost"
          size="icon"
          className="absolute right-2 top-2 h-8 w-8 text-text-tertiary hover:text-foreground"
          onClick={onCopy}
          aria-label={translateUi("Copy")}
        >
          <Copy className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

function EmptyState() {
  useUiTranslation();
  return (
    <div className="flex flex-col items-center gap-1 py-12">
      <BrainIcon className="h-48 w-48 text-muted-foreground" />
      <p className="text-14 text-muted-foreground">{translateUi("Create an OAuth client first to generate AI setup configs.")}</p>
    </div>
  );
}


function AiSkillSetupCard({
  baseUrl,
  dashboardUrl,
}: {
  readonly baseUrl: string;
  readonly dashboardUrl: string;
}) {
  useUiTranslation();
  const searchParams = useRouterState({
    select: (s) => s.location.search as Record<string, unknown>,
  });
  const navigate = useNavigate();
  const selectedSkillTool: AiTool = parseTab(
    searchParams.skill,
    AI_SETUP_SKILL_TABS,
    AI_SETUP_SKILL_TAB_DEFAULT,
  );

  function setSelectedSkillTool(value: AiTool) {
    void navigate({
      to: "/ai-setup",
      search: (prev) => ({ ...prev, skill: value }),
      replace: true,
    });
  }

  const setupPrompt = useMemo(
    () => generateSetupPrompt(selectedSkillTool, { baseUrl, dashboardUrl }),
    [selectedSkillTool, baseUrl, dashboardUrl],
  );

  const skillInfo = AI_TOOL_SKILL_INFO[selectedSkillTool];

  const handleCopy = useCallback(async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(translateUi("Copied to clipboard"));
    } catch {
      toast.error(translateErrorMessage("Failed to copy"));
    }
  }, []);

  return (
    <Card>
      <CardHeader>
        <CardTitle>{translateUi("Install AI Skills")}</CardTitle>
        <CardDescription>{translateUi("Install persistent NyxID skills so your AI agent automatically knows about NyxID in every session. No need to paste a prompt each time.")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <Tabs
          value={selectedSkillTool}
          onValueChange={(v) => setSelectedSkillTool(v as AiTool)}
          className="space-y-4"
        >
          <TabsList>
            {AI_SETUP_SKILL_TABS.map((id) => {
              const tool = AI_TOOLS.find((t) => t.id === id);
              return tool ? (
                <TabsTrigger key={id} value={id}>
                  {tool.name}
                </TabsTrigger>
              ) : null;
            })}
          </TabsList>

          {AI_SETUP_SKILL_TABS.map((id) => (
            <TabsContent key={id} value={id}>
              <div className="space-y-4">
                <div className="relative">
                  <pre className="whitespace-pre-wrap break-all rounded-lg border border-border bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-14">
                    {setupPrompt}
                  </pre>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="absolute right-2 top-2 h-8 w-8 text-text-tertiary hover:text-foreground"
                    onClick={() => void handleCopy(setupPrompt)}
                    aria-label={translateUi("Copy")}
                  >
                    <Copy className="h-3.5 w-3.5" />
                  </Button>
                </div>

                <div className="flex items-center gap-2">
                  <Badge
                    variant={
                      skillInfo.type === "auto-refresh" ? "default" : "secondary"
                    }
                    className="text-12"
                  >
                    {skillInfo.type === "auto-refresh"
                      ? translateUi("Auto-refresh")
                      : skillInfo.type === "manual-refresh"
                        ? translateUi("Manual refresh")
                        : skillInfo.type === "provider-level"
                          ? translateUi("Provider-level")
                          : translateUi("Per-session")}
                  </Badge>
                  <span className="text-13 text-muted-foreground">
                    {skillInfo.note}
                  </span>
                </div>
              </div>
            </TabsContent>
          ))}
        </Tabs>

        <p className="text-13 text-muted-foreground">{translateUi("Skills are powered by the NyxID playbook at")}{" "}
          <code className="rounded bg-muted px-1 py-0.5 text-14">
            {baseUrl}/llms.txt
          </code>
          . Tools with auto-refresh fetch the latest version automatically.
          Check skill status:{" "}
          <code className="rounded bg-muted px-1 py-0.5 text-14">
            nyxid ai-setup status
          </code>
          . Update the CLI by re-running the installer:{" "}
          <code className="rounded bg-muted px-1 py-0.5 text-14">
            bash -c &quot;$(curl -fsSL ...install.sh)&quot;
          </code>
        </p>
      </CardContent>
    </Card>
  );
}


export function AiSetupPage() {
  useUiTranslation();
  const { data: appsData, isLoading: appsLoading } = useDeveloperApps();
  const { data: config, isLoading: configLoading } = usePublicConfig();

  const searchParams = useRouterState({
    select: (s) => s.location.search as Record<string, unknown>,
  });
  const navigate = useNavigate();

  const [selectedClientId, setSelectedClientId] = useState<string>("");

  // AI_TOOLS lives in ai-tool-configs as the canonical source of all
  // tool ids + metadata; we derive the URL allowlist from it rather than
  // duplicating the list in url-tabs.
  const aiToolIds: readonly AiTool[] = AI_TOOLS.map((t) => t.id);
  const selectedTool: AiTool = isValidTab(searchParams.tool, aiToolIds)
    ? searchParams.tool
    : "cursor";

  function setSelectedTool(value: AiTool) {
    void navigate({
      to: "/ai-setup",
      search: (prev) => ({ ...prev, tool: value }),
      replace: true,
    });
  }

  const clients = appsData?.clients ?? [];
  const mcpUrl = config?.mcp_url ?? `${window.location.origin}/mcp`;
  const baseUrl = mcpUrl.replace(/\/mcp$/, "");

  const selectedClient = useMemo(
    () => clients.find((c) => c.id === selectedClientId) ?? clients[0] ?? null,
    [clients, selectedClientId],
  );

  const configParams: AiToolConfigParams | null = useMemo(() => {
    if (!selectedClient) return null;
    return {
      baseUrl,
      mcpUrl,
      clientId: selectedClient.id,
      clientName: selectedClient.client_name,
      redirectUris: selectedClient.redirect_uris,
    };
  }, [baseUrl, mcpUrl, selectedClient]);

  const generatedConfig = useMemo(() => {
    if (!configParams) return "";
    return generateToolConfig(selectedTool, configParams);
  }, [selectedTool, configParams]);

  const toolMeta = AI_TOOLS.find((t) => t.id === selectedTool)!;

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(generatedConfig);
      toast.success(translateUi("Copied to clipboard"));
    } catch {
      toast.error(translateErrorMessage("Failed to copy to clipboard"));
    }
  }, [generatedConfig]);

  if (appsLoading || configLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-32 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <div>
        <h2 className="type-page-title tracking-tight" style={{ letterSpacing: "-0.03em" }}>{translateUi("AI Setup Guide")}</h2>
        <p className="text-14 text-muted-foreground">{translateUi("Configure your AI coding assistant to work with NyxID. Pick a tool and an app, copy the config, done.")}</p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-4 p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-start gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-primary/25 bg-primary/10">
              <Shield className="h-4 w-4 text-primary-text" />
            </div>
            <div className="space-y-1">
              <h3 className="type-card-title text-foreground">{translateUi("Isolate this AI agent before handing it credentials")}</h3>
              <p className="max-w-2xl text-13 text-muted-foreground">{translateUi("Create a scoped Agent Key, choose allowed services, add credential bindings only when needed, then verify proxy access from the Agent Key detail page.")}</p>
            </div>
          </div>
          <Button variant="primary" asChild className="shrink-0">
            <Link to="/keys" search={{ tab: "nyxid", action: "setup-agent" }}>
              <ButtonIcon variant="primary"><Shield className="h-3 w-3" /></ButtonIcon>{translateUi("Set Up Agent Key")}</Link>
          </Button>
        </CardContent>
      </Card>

      <AiSkillSetupCard baseUrl={baseUrl} dashboardUrl={window.location.origin} />

      <Separator />

      {clients.length === 0 ? (
        <div className="space-y-6">
          <div className="flex justify-end">
            <Button asChild variant="outline" className="text-text-tertiary hover:text-muted-foreground">
              <Link to="/developer/apps">
                <ButtonIcon><Plus className="h-3 w-3" /></ButtonIcon>{translateUi("Create Developer App")}</Link>
            </Button>
          </div>
          <EmptyState />
        </div>
      ) : (
        <div className="space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>{translateUi("MCP Config Generator")}</CardTitle>
              <CardDescription>{translateUi("Select your OAuth client and AI tool to generate a ready-to-paste configuration file.")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              {/* App selector */}
              <div className="space-y-2">
                <Label>{translateUi("OAuth Client")}</Label>
                <Select
                  value={selectedClient?.id ?? ""}
                  onValueChange={setSelectedClientId}
                >
                  <SelectTrigger className="w-full max-w-md">
                    <SelectValue placeholder={translateUi("Select an app")} />
                  </SelectTrigger>
                  <SelectContent>
                    {clients.map((client) => (
                      <SelectItem key={client.id} value={client.id}>
                        <span className="flex items-center gap-2">
                          {client.client_name}
                          <Badge variant="secondary" className="text-12">
                            {client.client_type}
                          </Badge>
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {/* Tool tabs */}
              <Tabs
                value={selectedTool}
                onValueChange={(v) => setSelectedTool(v as AiTool)}
                className="space-y-4"
              >
                <TabsList>
                  {AI_TOOLS.map((tool) => (
                    <TabsTrigger key={tool.id} value={tool.id}>
                      {tool.name}
                    </TabsTrigger>
                  ))}
                </TabsList>

                {AI_TOOLS.map((tool) => (
                  <TabsContent key={tool.id} value={tool.id}>
                    <div className="space-y-4">
                      {tool.configFilePath && (
                        <p className="text-14 text-muted-foreground">{translateUi("Save to")}{" "}
                          <code className="rounded bg-muted px-1.5 py-0.5 text-14">
                            {tool.configFilePath}
                          </code>
                          {tool.id === "claude-code" && (
                            <span>
                              {" "}{translateUi("or project-level")}{" "}
                              <code className="rounded bg-muted px-1.5 py-0.5 text-14">
                                .claude/settings.json
                              </code>
                            </span>
                          )}
                        </p>
                      )}
                      <CodeBlock
                        code={generatedConfig}
                        label={toolMeta.configFileName}
                        onCopy={handleCopy}
                      />
                    </div>
                  </TabsContent>
                ))}
              </Tabs>
            </CardContent>
          </Card>

          {/* Context for the selected client */}
          {selectedClient && (
            <Card>
              <CardHeader>
                <CardTitle>{translateUi("Client Details")}</CardTitle>
              </CardHeader>
              <CardContent>
                <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-14">
                  <dt className="text-muted-foreground">{translateUi("Client ID")}</dt>
                  <dd className="text-13">{selectedClient.id}</dd>
                  <dt className="text-muted-foreground">{translateUi("Type")}</dt>
                  <dd>{selectedClient.client_type}</dd>
                  <dt className="text-muted-foreground">{translateUi("Redirect URIs")}</dt>
                  <dd className="text-13">
                    {selectedClient.redirect_uris.join(", ") || translateUi("None")}
                  </dd>
                  <dt className="text-muted-foreground">{translateUi("Scopes")}</dt>
                  <dd className="text-13">
                    {selectedClient.allowed_scopes || "openid profile email"}
                  </dd>
                </dl>
              </CardContent>
            </Card>
          )}
        </div>
      )}
    </div>
  );
}
