import { useState } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import { useConsentStore } from "@/stores/consent-store";
import { useUser, useMfaDisable } from "@/hooks/use-auth";
import { api, ApiError } from "@/lib/api-client";
import { disableTelemetry } from "@/lib/telemetry";
import type { User, Session } from "@/types/api";
import {
  changePasswordSchema,
  type ChangePasswordFormData,
} from "@/schemas/auth";
import { copyToClipboard, formatDate } from "@/lib/utils";
import { openExternal } from "@/lib/navigation";
import {
  SETTINGS_TABS,
  SETTINGS_TAB_DEFAULT,
  parseTab,
} from "@/lib/url-tabs";
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
import { Button, ButtonIcon } from "@/components/ui/button";
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
  Trash2,
  ExternalLink,
  Copy,
  Check,
  Eye,
  EyeOff,
} from "lucide-react";
import { PowerButtonIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";
import {
  getDeviceIcon,
  buildCursorDeeplink,
  buildClaudeCodeCommand,
  buildCursorConfig,
  buildClaudeCodeConfig,
  buildCodexCommand,
  buildCodexConfig,
} from "./settings.helpers";

interface DeleteAccountResponse {
  readonly status: string;
  readonly deleted_at: string;
}

export function SettingsPage() {
  const searchParams = useRouterState({ select: (s) => s.location.search as Record<string, unknown> });
  const navigate = useNavigate();
  const currentTab = parseTab(searchParams.tab, SETTINGS_TABS, SETTINGS_TAB_DEFAULT);

  function handleTabChange(value: string) {
    void navigate({ to: "/settings", search: { tab: value }, replace: true });
  }

  return (
    <div className="space-y-8">
      <div>
        <h2 className="text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
          Account Settings
        </h2>
        <p className="text-[12px] text-muted-foreground">
          Manage your account settings and preferences.
        </p>
      </div>

      <Tabs value={currentTab} onValueChange={handleTabChange} className="space-y-6">
        <TabsList>
          <TabsTrigger value="profile">Profile</TabsTrigger>
          <TabsTrigger value="security">Security</TabsTrigger>
          <TabsTrigger value="sessions">Sessions</TabsTrigger>
          <TabsTrigger value="mcp">MCP</TabsTrigger>
          <TabsTrigger value="privacy">Privacy</TabsTrigger>
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
        <TabsContent value="privacy">
          <PrivacyTab />
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

  const displayName = name || user?.display_name || "";

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
          <label className="text-[12px] font-medium" htmlFor="profile-name">
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
          <div className="flex items-center justify-between">
            <label className="text-[12px] font-medium" htmlFor="profile-email">
              Email
            </label>
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
          <Input
            id="profile-email"
            value={user?.email ?? ""}
            disabled
            className="opacity-50"
            aria-readonly="true"
          />
          {!user?.email_verified && (
            <Button
              type="button"
              variant="link"
              size="sm"
              className="h-auto p-0 text-[12px]"
              onClick={() => {
                // TODO: backend endpoint POST /api/v1/auth/verify-email/resend is not yet
                // implemented. Do NOT invent the URL — once it lands, wire it here as:
                //   await api.post<void>("/auth/verify-email/resend");
                //   toast.success("Verification email sent");
                toast.info(
                  "Email resend is not yet available on this server.",
                );
              }}
            >
              Resend verification email
            </Button>
          )}
        </div>
      </CardContent>
      <CardFooter>
        <Button variant="primary" onClick={() => void handleSave()} isLoading={saving} disabled={!name || name === (user?.display_name ?? "")}>
          Save Changes
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

  const [showCurrentPw, setShowCurrentPw] = useState(false);
  const [showNewPw, setShowNewPw] = useState(false);
  const [showConfirmPw, setShowConfirmPw] = useState(false);

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
      } catch {
        // Logout may fail if session is already invalidated by account deletion
      }
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
          <CardTitle>Two-Factor Authentication</CardTitle>
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
              <span className="text-[12px]">
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
                className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive"
              >
                {disableMfaError}
              </div>
            )}
            <div className="space-y-2">
              <label
                className="text-[12px] font-medium"
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
                  className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive"
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
                      <div className="relative">
                        <Input
                          type={showCurrentPw ? "text" : "password"}
                          autoComplete="current-password"
                          className="pr-10"
                          {...field}
                        />
                        <button
                          type="button"
                          tabIndex={-1}
                          onClick={() => setShowCurrentPw((v) => !v)}
                          className="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
                          aria-label={showCurrentPw ? "Hide password" : "Show password"}
                        >
                          {showCurrentPw ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}
                        </button>
                      </div>
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
                      <div className="relative">
                        <Input
                          type={showNewPw ? "text" : "password"}
                          autoComplete="new-password"
                          className="pr-10"
                          {...field}
                        />
                        <button
                          type="button"
                          tabIndex={-1}
                          onClick={() => setShowNewPw((v) => !v)}
                          className="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
                          aria-label={showNewPw ? "Hide password" : "Show password"}
                        >
                          {showNewPw ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}
                        </button>
                      </div>
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
                      <div className="relative">
                        <Input
                          type={showConfirmPw ? "text" : "password"}
                          autoComplete="new-password"
                          className="pr-10"
                          {...field}
                        />
                        <button
                          type="button"
                          tabIndex={-1}
                          onClick={() => setShowConfirmPw((v) => !v)}
                          className="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
                          aria-label={showConfirmPw ? "Hide password" : "Show password"}
                        >
                          {showConfirmPw ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}
                        </button>
                      </div>
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <div className="flex justify-end">
                <Button
                  variant="primary"
                  type="submit"
                  isLoading={passwordForm.formState.isSubmitting}
                  disabled={!passwordForm.formState.isDirty}
                >
                  Change Password
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>

      <Card className="border-destructive/40">
        <CardHeader className="flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
          <div className="space-y-1.5">
            <CardTitle className="text-destructive">Delete Account</CardTitle>
            <CardDescription className="text-destructive/70">
              Permanently delete your account and all associated data. This action
              cannot be undone.
            </CardDescription>
          </div>
          <Button
            variant="destructive"
            className="shrink-0"
            onClick={() => setDeleteAccountOpen(true)}
          >
            <ButtonIcon variant="destructive"><Trash2 className="h-3 w-3 text-destructive" /></ButtonIcon>
            Delete My Account
          </Button>
        </CardHeader>
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
            <label
              className="text-[12px] font-medium"
              htmlFor="delete-account-email"
            >
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

// ---------------------------------------------------------------------------
// MCP Install Tab
// ---------------------------------------------------------------------------

function CopyInlineButton({
  text,
  label,
}: {
  readonly text: string;
  readonly label: string;
}) {
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
      <div>
        <h3 className="text-lg font-medium">MCP (Model Context Protocol)</h3>
        <p className="text-[12px] text-muted-foreground">
          Install NyxID as an MCP server in your client of choice.
        </p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>Install to Cursor</CardTitle>
          <CardDescription>
            One-click install via Cursor's deeplink protocol. Cursor will open
            and prompt you to confirm the MCP server installation.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <Button
            variant="primary"
            onClick={() => void openExternal(cursorDeeplink)}
            className="w-full"
          >
            <ButtonIcon><ExternalLink className="h-4 w-4" /></ButtonIcon>
            Install to Cursor
          </Button>
          <Separator />
          <div>
            <div className="mb-1 flex items-center gap-2">
              <p className="text-xs font-medium text-muted-foreground">
                Or copy manually
              </p>
              <Badge variant="secondary" className="text-[10px]">
                .cursor/mcp.json
              </Badge>
            </div>
            <div className="relative">
              <pre className="whitespace-pre-wrap break-all rounded-lg border border-border bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-xs leading-relaxed">
                {cursorConfig}
              </pre>
              <CopyInlineButton text={cursorConfig} label="Cursor config" />
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Install to Claude Code</CardTitle>
          <CardDescription>
            Run this command in your terminal to add NyxID as an MCP server in
            Claude Code.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="relative">
            <code className="block rounded-lg border border-border bg-muted px-4 py-3 pr-10 text-xs break-all font-mono">
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
              <Badge variant="secondary" className="text-[10px]">
                .claude/settings.json or .mcp.json
              </Badge>
            </div>
            <div className="relative">
              <pre className="whitespace-pre-wrap break-all rounded-lg border border-border bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-xs leading-relaxed">
                {claudeConfig}
              </pre>
              <CopyInlineButton
                text={claudeConfig}
                label="Claude Code config"
              />
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Install to Codex</CardTitle>
          <CardDescription>
            Run this command in your terminal to add NyxID as an MCP server in
            Codex CLI.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="relative">
            <code className="block rounded-lg border border-border bg-muted px-4 py-3 pr-10 text-xs break-all font-mono">
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
              <Badge variant="secondary" className="text-[10px]">
                ~/.codex/config.toml
              </Badge>
            </div>
            <div className="relative">
              <pre className="whitespace-pre-wrap break-all rounded-lg border border-border bg-muted px-4 py-3.5 pr-10 min-h-[44px] font-mono text-xs leading-relaxed">
                {codexConfig}
              </pre>
              <CopyInlineButton text={codexConfig} label="Codex config" />
            </div>
          </div>
        </CardContent>
      </Card>

      <div className="rounded-xl border border-border/50 bg-muted/30 p-4">
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
          View your active sessions across devices.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {!sessions || sessions.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
            <PowerButtonIcon className="h-48 w-48 text-muted-foreground" />
            <p className="text-[12px] font-medium text-muted-foreground">No Active Sessions</p>
            <p className="text-[12px] text-muted-foreground">
              Your active sessions across devices will appear here.
            </p>
          </div>
        ) : (
          <>
          {/* Mobile card view */}
          <div className="flex flex-col gap-3 md:hidden">
            {sessions.map((session) => (
              <div
                key={session.id}
                className="rounded-xl border border-border/50 bg-card p-4"
              >
                <div className="flex items-center gap-2">
                  {getDeviceIcon(session.user_agent)}
                  <p className="min-w-0 flex-1 truncate text-[13px] font-bold">
                    {session.user_agent}
                  </p>
                </div>
                <div className="mt-3 space-y-1">
                  <p className="text-[11px] text-muted-foreground">
                    <span className="font-medium">IP Address:</span>{" "}
                    {session.ip_address}
                  </p>
                  <p className="text-[11px] text-muted-foreground">
                    <span className="font-medium">Created:</span>{" "}
                    {formatDate(session.created_at)}
                  </p>
                  <p className="text-[11px] text-muted-foreground">
                    <span className="font-medium">Expires:</span>{" "}
                    {formatDate(session.expires_at)}
                  </p>
                </div>
              </div>
            ))}
          </div>

          {/* Desktop table view */}
          <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
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
                        <span className="max-w-[200px] truncate">
                          {session.user_agent}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell>
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
          </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}

function PrivacyTab() {
  const consentEnabled = useConsentStore((s) => s.enabled);
  const consentAsked = useConsentStore((s) => s.asked);
  const setConsent = useConsentStore((s) => s.setConsent);

  // When the user flips OFF here after a prior opt-in, tear down the
  // running PostHog client immediately so no further events fire even
  // while the tab stays open. The `useEffect` in main.tsx that drives
  // `initTelemetry` already re-runs whenever `enabled` changes, so the
  // ON path only needs to flip the store — main.tsx picks it up and
  // re-initializes cleanly against the `inited` guard.
  function handleToggle(next: boolean) {
    setConsent(next);
    if (!next) {
      disableTelemetry();
    }
  }

  const statusLabel = !consentAsked
    ? "Not yet answered"
    : consentEnabled
      ? "Enabled"
      : "Disabled";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Anonymous Usage Telemetry</CardTitle>
        <CardDescription>
          Help us improve NyxID by sharing anonymous usage events. We never
          capture credentials, form content, or the contents of your
          requests. This choice applies to this browser only — other
          devices and the CLI manage their own telemetry settings.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-3">
          <Switch
            checked={consentEnabled}
            onCheckedChange={handleToggle}
            aria-label="Toggle anonymous usage telemetry"
          />
          <span className="text-[12px]">{statusLabel}</span>
        </div>
        <Separator />
        <div className="space-y-2 text-[12px] text-muted-foreground">
          <p>
            For the full disclosure of what we collect, how it's stored, and
            retention windows, see the{" "}
            <a
              href="/privacy"
              className="underline underline-offset-2 hover:text-foreground"
            >
              privacy policy
            </a>
            .
          </p>
        </div>
      </CardContent>
    </Card>
  );
}
