import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import { useUser, useMfaDisable } from "@/hooks/use-auth";
import { api, ApiError } from "@/lib/api-client";
import type { User, Session } from "@/types/api";
import {
  changePasswordSchema,
  type ChangePasswordFormData,
} from "@/schemas/auth";
import { copyToClipboard, formatDate } from "@/lib/utils";
import { openExternal } from "@/lib/navigation";
import { usePublicConfig } from "@/hooks/use-public-config";
import { MfaSetupDialog } from "@/components/auth/mfa-setup-dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  ShieldCheck,
  ShieldOff,
  TriangleAlert,
  Monitor,
  Smartphone,
  Globe,
  Terminal,
  ExternalLink,
  Copy,
  Check,
} from "lucide-react";
import { toast } from "sonner";

interface DeleteAccountResponse {
  readonly status: string;
  readonly deleted_at: string;
}

export function SettingsPage() {
  return (
    <div className="space-y-8">
      <div>
        <h2 className="font-display text-3xl md:text-5xl font-normal tracking-tight">Settings</h2>
        <p className="text-muted-foreground">
          Manage your account settings and preferences.
        </p>
      </div>

      <Tabs defaultValue="profile" className="space-y-6">
        <TabsList>
          <TabsTrigger value="profile">Profile</TabsTrigger>
          <TabsTrigger value="security">Security</TabsTrigger>
          <TabsTrigger value="sessions">Sessions</TabsTrigger>
          <TabsTrigger value="mcp">MCP</TabsTrigger>
        </TabsList>

        <TabsContent value="profile">
          <ProfileTab />
        </TabsContent>
        <TabsContent value="security">
          <SecurityTab />
        </TabsContent>
        <TabsContent value="sessions">
          <SessionsTab />
        </TabsContent>
        <TabsContent value="mcp">
          <McpTab />
        </TabsContent>
      </Tabs>
    </div>
  );
}

function ProfileTab() {
  const { data: user, isLoading } = useUser();
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const setUser = useAuthStore((s) => s.setUser);

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  const displayName = name || user?.name || "";

  async function handleSave() {
    setSaving(true);
    try {
      const updated = await api.put<User>("/users/me", {
        display_name: displayName,
      });
      setUser(updated);
      toast.success("Profile updated successfully");
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to update profile");
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Profile</CardTitle>
        <CardDescription>Update your personal information.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <label className="text-sm font-medium" htmlFor="profile-name">
            Name
          </label>
          <Input
            id="profile-name"
            value={displayName}
            onChange={(e) => setName(e.target.value)}
            placeholder="Your name"
          />
        </div>
        <div className="space-y-2">
          <label className="text-sm font-medium" htmlFor="profile-email">
            Email
          </label>
          <Input
            id="profile-email"
            value={user?.email ?? ""}
            disabled
            className="opacity-50"
            aria-readonly="true"
          />
          <div>
            {user?.email_verified ? (
              <Badge variant="success" className="text-xs">
                Verified
              </Badge>
            ) : (
              <Badge variant="warning" className="text-xs">
                Not verified
              </Badge>
            )}
          </div>
        </div>
      </CardContent>
      <CardFooter>
        <Button onClick={() => void handleSave()} isLoading={saving}>
          Save changes
        </Button>
      </CardFooter>
    </Card>
  );
}

function SecurityTab() {
  const user = useAuthStore((s) => s.user);
  const logout = useAuthStore((s) => s.logout);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [mfaDialogOpen, setMfaDialogOpen] = useState(false);
  const [disableMfaDialogOpen, setDisableMfaDialogOpen] = useState(false);
  const [disableMfaPassword, setDisableMfaPassword] = useState("");
  const [disableMfaError, setDisableMfaError] = useState<string | null>(null);
  const [deleteAccountOpen, setDeleteAccountOpen] = useState(false);
  const [deleteConfirmEmail, setDeleteConfirmEmail] = useState("");
  const [isDeletingAccount, setIsDeletingAccount] = useState(false);
  const disableMfa = useMfaDisable();

  const passwordForm = useForm<ChangePasswordFormData>({
    resolver: zodResolver(changePasswordSchema),
    defaultValues: {
      currentPassword: "",
      newPassword: "",
      confirmNewPassword: "",
    },
  });

  async function handleChangePassword(data: ChangePasswordFormData) {
    try {
      await api.post<void>("/auth/password/change", {
        current_password: data.currentPassword,
        new_password: data.newPassword,
      });
      toast.success("Password changed successfully");
      passwordForm.reset();
    } catch (error) {
      if (error instanceof ApiError) {
        passwordForm.setError("root", { message: error.message });
      } else {
        toast.error("Failed to change password");
      }
    }
  }

  async function handleDisableMfa() {
    if (!disableMfaPassword) {
      setDisableMfaError("Password is required");
      return;
    }
    try {
      await disableMfa.mutateAsync(disableMfaPassword);
      toast.success("MFA disabled");
      setDisableMfaDialogOpen(false);
      setDisableMfaPassword("");
      setDisableMfaError(null);
    } catch (error) {
      if (error instanceof ApiError) {
        setDisableMfaError(error.message);
      } else {
        setDisableMfaError("Failed to disable MFA");
      }
    }
  }

  function handleDisableMfaClose() {
    setDisableMfaDialogOpen(false);
    setDisableMfaPassword("");
    setDisableMfaError(null);
  }

  async function handleDeleteAccount() {
    if (!user?.email) return;
    if (deleteConfirmEmail.trim() !== user.email) {
      toast.error("Please enter your email to confirm");
      return;
    }
    setIsDeletingAccount(true);
    try {
      await api.delete<DeleteAccountResponse>("/users/me");
      toast.success("Account deleted successfully");
      try {
        await logout();
      } catch {}
      queryClient.clear();
      void navigate({ to: "/login" });
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to delete account");
      }
    } finally {
      setIsDeletingAccount(false);
    }
  }

  function handleDeleteAccountDialog(open: boolean) {
    setDeleteAccountOpen(open);
    if (!open) {
      setDeleteConfirmEmail("");
    }
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            {user?.mfa_enabled ? (
              <ShieldCheck
                className="h-5 w-5 text-success"
                aria-hidden="true"
              />
            ) : (
              <ShieldOff
                className="h-5 w-5 text-muted-foreground"
                aria-hidden="true"
              />
            )}
            Two-Factor Authentication
          </CardTitle>
          <CardDescription>
            {user?.mfa_enabled
              ? "Your account is protected with two-factor authentication."
              : "Add an extra layer of security to your account."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Switch
                checked={user?.mfa_enabled ?? false}
                onCheckedChange={(checked) => {
                  if (checked) {
                    setMfaDialogOpen(true);
                  } else {
                    setDisableMfaDialogOpen(true);
                  }
                }}
                aria-label="Toggle two-factor authentication"
              />
              <span className="text-sm">
                {user?.mfa_enabled ? "Enabled" : "Disabled"}
              </span>
            </div>
          </div>
        </CardContent>
      </Card>

      <MfaSetupDialog open={mfaDialogOpen} onOpenChange={setMfaDialogOpen} />

      <Dialog open={disableMfaDialogOpen} onOpenChange={handleDisableMfaClose}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Disable Two-Factor Authentication</DialogTitle>
            <DialogDescription>
              Enter your password to confirm disabling two-factor
              authentication. This will make your account less secure.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {disableMfaError && (
              <div
                role="alert"
                className="rounded-md bg-destructive/10 p-3 text-sm text-destructive"
              >
                {disableMfaError}
              </div>
            )}
            <div className="space-y-2">
              <label
                className="text-sm font-medium"
                htmlFor="disable-mfa-password"
              >
                Password
              </label>
              <Input
                id="disable-mfa-password"
                type="password"
                autoComplete="current-password"
                value={disableMfaPassword}
                onChange={(e) => setDisableMfaPassword(e.target.value)}
                placeholder="Enter your password"
                autoFocus
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={handleDisableMfaClose}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDisableMfa()}
              isLoading={disableMfa.isPending}
            >
              Disable MFA
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Card>
        <CardHeader>
          <CardTitle>Change Password</CardTitle>
          <CardDescription>
            Update your password to keep your account secure.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Form {...passwordForm}>
            <form
              onSubmit={passwordForm.handleSubmit(handleChangePassword)}
              className="space-y-4"
            >
              {passwordForm.formState.errors.root && (
                <div
                  role="alert"
                  className="rounded-md bg-destructive/10 p-3 text-sm text-destructive"
                >
                  {passwordForm.formState.errors.root.message}
                </div>
              )}

              <FormField
                control={passwordForm.control}
                name="currentPassword"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Current Password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        autoComplete="current-password"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <Separator />

              <FormField
                control={passwordForm.control}
                name="newPassword"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>New Password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        autoComplete="new-password"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={passwordForm.control}
                name="confirmNewPassword"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Confirm New Password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        autoComplete="new-password"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <Button
                type="submit"
                isLoading={passwordForm.formState.isSubmitting}
              >
                Change password
              </Button>
            </form>
          </Form>
        </CardContent>
      </Card>

      <Card className="border-destructive/40">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-destructive">
            <TriangleAlert className="h-5 w-5" aria-hidden="true" />
            Delete Account
          </CardTitle>
          <CardDescription>
            Permanently delete your account and all associated data. This action
            cannot be undone.
          </CardDescription>
        </CardHeader>
        <CardFooter>
          <Button
            variant="destructive"
            onClick={() => setDeleteAccountOpen(true)}
          >
            Delete My Account
          </Button>
        </CardFooter>
      </Card>

      <Dialog open={deleteAccountOpen} onOpenChange={handleDeleteAccountDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Account</DialogTitle>
            <DialogDescription>
              Please enter your email address to confirm account deletion. This
              action cannot be undone.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <label className="text-sm font-medium" htmlFor="delete-account-email">
              Email
            </label>
            <Input
              id="delete-account-email"
              value={deleteConfirmEmail}
              onChange={(e) => setDeleteConfirmEmail(e.target.value)}
              placeholder={user?.email ?? "your@email.com"}
              autoFocus
            />
          </div>

          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => handleDeleteAccountDialog(false)}
              disabled={isDeletingAccount}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              isLoading={isDeletingAccount}
              disabled={deleteConfirmEmail.trim() !== (user?.email ?? "")}
              onClick={() => void handleDeleteAccount()}
            >
              Permanently Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function getDeviceIcon(userAgent: string) {
  const ua = userAgent.toLowerCase();
  if (
    ua.includes("mobile") ||
    ua.includes("android") ||
    ua.includes("iphone")
  ) {
    return <Smartphone className="h-4 w-4" aria-hidden="true" />;
  }
  if (
    ua.includes("mozilla") ||
    ua.includes("chrome") ||
    ua.includes("safari")
  ) {
    return <Monitor className="h-4 w-4" aria-hidden="true" />;
  }
  return <Globe className="h-4 w-4" aria-hidden="true" />;
}

// ---------------------------------------------------------------------------
// MCP Install Tab
// ---------------------------------------------------------------------------

function buildCursorDeeplink(mcpUrl: string): string {
  const config = JSON.stringify({ url: mcpUrl });
  const encoded = encodeURIComponent(btoa(config));
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=nyxid&config=${encoded}`;
}

function buildClaudeCodeCommand(mcpUrl: string): string {
  return `claude mcp add --transport http nyxid ${mcpUrl}`;
}

function buildCursorConfig(mcpUrl: string): string {
  return JSON.stringify(
    { mcpServers: { nyxid: { url: mcpUrl } } },
    null,
    2,
  );
}

function buildClaudeCodeConfig(mcpUrl: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        nyxid: {
          command: "npx",
          args: ["-y", "@anthropic-ai/mcp-proxy", mcpUrl],
        },
      },
    },
    null,
    2,
  );
}

function buildCodexCommand(mcpUrl: string): string {
  return `codex mcp add nyxid --url ${mcpUrl}`;
}

function buildCodexConfig(mcpUrl: string): string {
  return `[mcp_servers.nyxid]\nurl = "${mcpUrl}"`;
}

function CopyInlineButton({ text, label }: { readonly text: string; readonly label: string }) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await copyToClipboard(text);
      setCopied(true);
      toast.success(`${label} copied to clipboard`);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  return (
    <Button
      variant="ghost"
      size="icon"
      className="absolute right-2 top-2 h-6 w-6"
      onClick={() => void handleCopy()}
    >
      {copied ? (
        <Check className="h-3 w-3 text-success" aria-hidden="true" />
      ) : (
        <Copy className="h-3 w-3" aria-hidden="true" />
      )}
      <span className="sr-only">Copy {label}</span>
    </Button>
  );
}

function McpTab() {
  const { data: config, isLoading } = usePublicConfig();

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  const mcpUrl = config?.mcp_url ?? `${window.location.origin}/mcp`;
  const cursorDeeplink = buildCursorDeeplink(mcpUrl);
  const claudeCommand = buildClaudeCodeCommand(mcpUrl);
  const cursorConfig = buildCursorConfig(mcpUrl);
  const claudeConfig = buildClaudeCodeConfig(mcpUrl);
  const codexCommand = buildCodexCommand(mcpUrl);
  const codexConfig = buildCodexConfig(mcpUrl);

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <ExternalLink className="h-5 w-5" aria-hidden="true" />
            Install to Cursor
          </CardTitle>
          <CardDescription>
            One-click install via Cursor's deeplink protocol. Cursor will open
            and prompt you to confirm the MCP server installation.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <Button
            onClick={() => void openExternal(cursorDeeplink)}
            className="w-full"
          >
            <ExternalLink className="mr-2 h-4 w-4" aria-hidden="true" />
            Install to Cursor
          </Button>
          <Separator />
          <div>
            <div className="mb-1 flex items-center gap-2">
              <p className="text-xs font-medium text-muted-foreground">
                Or copy manually
              </p>
              <Badge variant="outline" className="text-[10px]">
                .cursor/mcp.json
              </Badge>
            </div>
            <div className="relative">
              <pre className="rounded-[10px] bg-muted px-3 py-2 pr-10 font-mono text-xs overflow-x-auto">
                {cursorConfig}
              </pre>
              <CopyInlineButton text={cursorConfig} label="Cursor config" />
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Terminal className="h-5 w-5" aria-hidden="true" />
            Install to Claude Code
          </CardTitle>
          <CardDescription>
            Run this command in your terminal to add NyxID as an MCP server in
            Claude Code.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="relative">
            <code className="block rounded-[10px] bg-muted px-3 py-2 pr-10 text-xs break-all font-mono">
              {claudeCommand}
            </code>
            <CopyInlineButton text={claudeCommand} label="CLI command" />
          </div>
          <Separator />
          <div>
            <div className="mb-1 flex items-center gap-2">
              <p className="text-xs font-medium text-muted-foreground">
                Or add manually
              </p>
              <Badge variant="outline" className="text-[10px]">
                .claude/settings.json
              </Badge>
            </div>
            <div className="relative">
              <pre className="rounded-[10px] bg-muted px-3 py-2 pr-10 font-mono text-xs overflow-x-auto">
                {claudeConfig}
              </pre>
              <CopyInlineButton text={claudeConfig} label="Claude Code config" />
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Terminal className="h-5 w-5" aria-hidden="true" />
            Install to Codex
          </CardTitle>
          <CardDescription>
            Run this command in your terminal to add NyxID as an MCP server in
            Codex CLI.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="relative">
            <code className="block rounded bg-muted px-3 py-2 pr-10 text-xs break-all font-mono">
              {codexCommand}
            </code>
            <CopyInlineButton text={codexCommand} label="CLI command" />
          </div>
          <Separator />
          <div>
            <div className="mb-1 flex items-center gap-2">
              <p className="text-xs font-medium text-muted-foreground">
                Or add manually
              </p>
              <Badge variant="outline" className="text-[10px]">
                ~/.codex/config.toml
              </Badge>
            </div>
            <div className="relative">
              <pre className="rounded bg-muted px-3 py-2 pr-10 text-xs overflow-x-auto">
                {codexConfig}
              </pre>
              <CopyInlineButton text={codexConfig} label="Codex config" />
            </div>
          </div>
        </CardContent>
      </Card>

      <div className="rounded-[10px] border border-border bg-muted/30 p-4">
        <p className="mb-1 text-[13px] font-medium">How it works</p>
        <p className="text-xs text-muted-foreground">
          When your MCP client connects for the first time, NyxID will open an
          OAuth flow in your browser to authenticate. Once authenticated, the
          proxy exposes tools from all your connected services. Tool calls are
          forwarded to each service's API with your credentials injected
          automatically.
        </p>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sessions Tab
// ---------------------------------------------------------------------------

function SessionsTab() {
  const { data: sessions, isLoading } = useQuery({
    queryKey: ["sessions"],
    queryFn: async (): Promise<readonly Session[]> => {
      return api.get<readonly Session[]>("/sessions");
    },
  });

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Active Sessions</CardTitle>
        <CardDescription>
          Manage your active sessions across devices.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {!sessions || sessions.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No active sessions found.
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Device</TableHead>
                <TableHead>IP Address</TableHead>
                <TableHead>Created</TableHead>
                <TableHead>Expires</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {sessions.map((session) => (
                <TableRow key={session.id}>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      {getDeviceIcon(session.user_agent)}
                      <span className="max-w-[200px] truncate text-sm">
                        {session.user_agent}
                      </span>
                    </div>
                  </TableCell>
                  <TableCell className="font-mono text-sm">
                    {session.ip_address}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(session.created_at)}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatDate(session.expires_at)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}
