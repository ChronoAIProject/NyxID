import { Label } from "@/components/ui/label";
import { withErasurePreparation } from "@/lib/erasure-preparation";
import { OwnedOnboardingBindings } from "@/pages/platform-onboarding";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { PageHeader } from "@/components/shared/page-header";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import { useConsentStore } from "@/stores/consent-store";
import { useUser, useMfaDisable, useRevokeSession } from "@/hooks/use-auth";
import { api, apiClient, ApiError } from "@/lib/api-client";
import { ACCOUNT_DELETION_POLICY_VERSION } from "@/lib/account-deletion-policy";
import { AccountDeletionDisclosure } from "@/components/shared/account-deletion-disclosure";
import { disableTelemetry } from "@/lib/telemetry";
import {
  readAnalyticsConsent,
  saveAnalyticsConsent,
} from "@/lib/analytics-consent";
import type { User, Session } from "@/types/api";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  changePasswordSchema,
  type ChangePasswordFormData,
} from "@/schemas/auth";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormLabel,
} from "@/components/ui/form";
import { copyToClipboard, formatDate } from "@/lib/utils";
import { openExternal } from "@/lib/navigation";
import { SETTINGS_TABS, SETTINGS_TAB_DEFAULT, parseTab } from "@/lib/url-tabs";
import { usePublicConfig } from "@/hooks/use-public-config";
import { MfaSetupDialog } from "@/components/auth/mfa-setup-dialog";
import { ApprovedSignInPolicy } from "@/components/settings/approved-sign-in-policy";
import { DisplaySettings } from "@/components/settings/display-settings";
import {
  SegmentedControl,
  SegmentedControlContent,
  SegmentedControlList,
  SegmentedControlTrigger,
} from "@/components/ui/segmented-control";
import {
  GroupedList,
  GroupedRow,
  GroupedInput as Input,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedMessage as FormMessage,
} from "@/components/ui/grouped-fields";
import {
  GroupedFormActions,
  GroupedFormBar,
  GroupedFormScope,
} from "@/components/ui/grouped-form";
import {
  GroupedPage,
  GroupedDialog as Dialog,
  GroupedDialogContent as DialogContent,
  GroupedDialogCancel,
  GroupedDecisionDraft,
} from "@/components/ui/grouped-dialog";
import { FormViewport } from "@/components/ui/form-viewport";
import { CompactChrome, CompactScroll } from "@/components/ui/compact-title";
import { describeChanges } from "@/lib/grouped-form-changes";
import {
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Eye, EyeOff, Trash2, ExternalLink, Copy, Check } from "lucide-react";
import { toast } from "sonner";
import {
  getDeviceIcon,
  humanizeUserAgent,
  humanizeIpAddress,
  buildCursorDeeplink,
  buildClaudeCodeCommand,
  buildCursorConfig,
  buildClaudeCodeConfig,
  buildCodexCommand,
  buildCodexConfig,
} from "./settings.helpers";

interface DeleteAccountResponse {
  readonly status: string;
  readonly accepted_at: string;
  readonly completed_at: string | null;
}

export function SettingsPage({
  presentation = "page",
}: { readonly presentation?: "page" | "panel" } = {}) {
  const content = <SettingsContent presentation={presentation} />;
  return presentation === "page" ? (
    <GroupedPage>{content}</GroupedPage>
  ) : (
    content
  );
}

function SettingsContent({
  presentation,
}: {
  readonly presentation: "page" | "panel";
}) {
  const searchParams = useRouterState({
    select: (s) => s.location.search as Record<string, unknown>,
  });
  const navigate = useNavigate();
  const account = useAccountPanel();
  const currentTab = parseTab(
    presentation === "panel" ? account.current.panelTab : searchParams.tab,
    SETTINGS_TABS,
    SETTINGS_TAB_DEFAULT,
  );
  const [visited, setVisited] = useState(() => new Set([currentTab]));
  if (!visited.has(currentTab)) setVisited(new Set([...visited, currentTab]));
  const panels = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (presentation !== "panel") return;
    const frame = requestAnimationFrame(() =>
      panels.current
        ?.querySelector<HTMLElement>(`[data-settings-tab="${currentTab}"] h2`)
        ?.focus({ preventScroll: true }),
    );
    return () => cancelAnimationFrame(frame);
  }, [currentTab, presentation]);

  function handleTabChange(value: string) {
    if (presentation === "panel") void account.update({ panelTab: value });
    else
      void navigate({ to: "/settings", search: { tab: value }, replace: true });
  }

  return (
    <GroupedFormScope viewport={presentation === "panel"}>
      <section
        aria-label="Account settings"
        className={
          presentation === "panel"
            ? "flex min-h-0 flex-1 flex-col"
            : "space-y-8"
        }
      >
        {presentation === "page" && (
          <PageHeader
            title="Account Settings"
            description="Manage your account settings and preferences."
          />
        )}
        <SegmentedControl
          value={currentTab}
          onValueChange={handleTabChange}
          className="flex min-h-0 flex-1 flex-col gap-6"
        >
          <CompactChrome>
            <SegmentedControlList aria-label="Account settings">
              {SETTINGS_TABS.map((tab) => (
                <SegmentedControlTrigger key={tab} value={tab}>
                  {tab === "mcp"
                    ? "MCP"
                    : tab.charAt(0).toUpperCase() + tab.slice(1)}
                </SegmentedControlTrigger>
              ))}
            </SegmentedControlList>
          </CompactChrome>
          <div
            ref={panels}
            className={
              presentation === "panel"
                ? "flex min-h-0 flex-1 flex-col"
                : undefined
            }
          >
            {SETTINGS_TABS.filter((tab) => visited.has(tab)).map((tab) => (
              <SegmentedControlContent
                key={tab}
                value={tab}
                forceMount
                hidden={currentTab !== tab}
                data-settings-tab={tab}
                className="min-h-0 flex-1 flex-col space-y-0 data-[state=active]:flex data-[state=inactive]:hidden"
              >
                <h2 tabIndex={-1} className="sr-only outline-none">
                  {tab === "mcp"
                    ? "MCP"
                    : tab.charAt(0).toUpperCase() + tab.slice(1)}
                </h2>
                <FormViewport className="flex min-h-0 flex-1 flex-col">
                  <CompactScroll
                    active={currentTab === tab}
                    className="assistant-scrollbar flex-1 space-y-6 pb-6"
                  >
                    {tab === "profile" ? (
                      <ProfileTab />
                    ) : tab === "security" ? (
                      <>
                        <SecurityTab />
                        <OwnedOnboardingBindings />
                      </>
                    ) : tab === "sessions" ? (
                      <SessionsTab />
                    ) : tab === "mcp" ? (
                      <McpTab />
                    ) : tab === "display" ? (
                      <DisplaySettings />
                    ) : (
                      <PrivacyTab />
                    )}
                  </CompactScroll>
                </FormViewport>
              </SegmentedControlContent>
            ))}
          </div>
        </SegmentedControl>
        <GroupedFormBar viewport={presentation === "panel"} />
      </section>
    </GroupedFormScope>
  );
}

function ProfileTab() {
  const { data: user, isLoading } = useUser();
  if (isLoading || !user) return <Skeleton className="h-32 w-full" />;
  return <ProfileForm key={user.id} user={user} />;
}

function ProfileForm({ user }: { readonly user: User }) {
  const form = useAppForm({
    defaultValues: { display_name: user.display_name ?? "" },
  });
  const setUser = useAuthStore((s) => s.setUser);
  const queryClient = useQueryClient();
  async function save(values: { display_name: string }) {
    form.clearErrors("root");
    try {
      const updated = await api.put<User>("/users/me", values);
      setUser(updated);
      queryClient.setQueryData(["user", "me"], updated);
      const draft = form.getValues("display_name");
      form.reset({ display_name: updated.display_name ?? "" });
      if (draft !== values.display_name) form.setValue("display_name", draft);
      toast.success("Profile updated successfully");
    } catch (error) {
      form.setError("root", {
        message:
          error instanceof ApiError
            ? error.message
            : "Failed to update profile",
      });
    }
  }
  return (
    <Form {...form}>
      <form
        aria-label="Profile"
        onSubmit={form.handleSubmit(save)}
        className="space-y-6"
      >
        <GroupedList
          title="Profile"
          footer={
            <>
              <p>Update your personal information.</p>
              {form.formState.errors.root && (
                <p role="alert" className="text-destructive">
                  {form.formState.errors.root.message}
                </p>
              )}
            </>
          }
        >
          <FormField
            control={form.control}
            name="display_name"
            rules={{ required: "Name is required" }}
            render={({ field }) => (
              <FormItem inline>
                <FormLabel>Name</FormLabel>
                <FormControl>
                  <Input
                    {...field}
                    className="min-w-0 flex-1 basis-40 sm:text-right"
                    placeholder="Your name"
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <GroupedRow>
            <Label htmlFor="profile-email" className="flex-1">
              Email
            </Label>
            <Badge variant={user.email_verified ? "success" : "warning"}>
              {user.email_verified ? "Verified" : "Not verified"}
            </Badge>
            <Input
              id="profile-email"
              value={user.email}
              disabled
              aria-readonly="true"
              className="basis-full text-text-tertiary disabled:opacity-100"
            />
          </GroupedRow>
          {!user.email_verified && (
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() =>
                  toast.info(
                    "Email resend is not yet available on this server.",
                  )
                }
              >
                Resend verification email
              </button>
            </GroupedRow>
          )}
        </GroupedList>
        <GroupedFormActions
          label="Profile"
          dirty={form.formState.isDirty}
          pending={form.formState.isSubmitting}
          disabled={!form.watch("display_name")}
          save={form.handleSubmit(save)}
          discard={() => form.reset()}
          changes={() =>
            describeChanges(form.formState.defaultValues, form.getValues(), [
              { key: "display_name", label: "Name" },
            ])
          }
        />
      </form>
    </Form>
  );
}

function SecurityTab() {
  const user = useAuthStore((s) => s.user);
  const logout = useAuthStore((s) => s.logout);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [mfaDialogOpen, setMfaDialogOpen] = useState(false);
  const [disableMfaDialogOpen, setDisableMfaDialogOpen] = useState(false);
  const [disableMfaCode, setDisableMfaCode] = useState("");
  const [disableMfaPassword, setDisableMfaPassword] = useState("");
  const [disableMfaError, setDisableMfaError] = useState<string | null>(null);
  const [deleteAccountOpen, setDeleteAccountOpen] = useState(false);
  const [deleteConfirmEmail, setDeleteConfirmEmail] = useState("");
  const [isDeletingAccount, setIsDeletingAccount] = useState(false);

  // Email comparison is case-insensitive (addresses are), and requires the
  // user to actually be loaded — otherwise an empty input would match an
  // empty `?? ""` fallback and arm the destructive button.
  const deleteEmailMatches =
    Boolean(user?.email) &&
    deleteConfirmEmail.trim().toLowerCase() === user?.email.toLowerCase();
  const deleteEmailError =
    deleteConfirmEmail.trim().length > 0 && !deleteEmailMatches
      ? "That does not match your account email."
      : null;
  const disableMfa = useMfaDisable();

  const [showCurrentPw, setShowCurrentPw] = useState(false);
  const [showNewPw, setShowNewPw] = useState(false);
  const [showConfirmPw, setShowConfirmPw] = useState(false);

  // MFA disable keeps the legacy credential inference for older servers.
  // The password-change card below requires explicit credential availability.
  const isPasswordUser = user?.has_password ?? !user?.social_provider;

  const passwordForm = useAppForm<ChangePasswordFormData>({
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
    if (!disableMfaCode.trim()) {
      setDisableMfaError("Enter your authenticator or recovery code");
      return;
    }
    if (isPasswordUser && !disableMfaPassword) {
      setDisableMfaError("Password is required for this account");
      return;
    }
    try {
      await disableMfa.mutateAsync({
        code: disableMfaCode.trim(),
        ...(isPasswordUser ? { password: disableMfaPassword } : {}),
      });
      toast.success("MFA disabled");
      setDisableMfaDialogOpen(false);
      setDisableMfaCode("");
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
    setDisableMfaCode("");
    setDisableMfaPassword("");
    setDisableMfaError(null);
  }

  async function handleDeleteAccount() {
    if (!user?.email) return;
    if (!deleteEmailMatches) {
      toast.error("Please enter your email to confirm");
      return;
    }
    setIsDeletingAccount(true);
    try {
      const result = await withErasurePreparation(() =>
        apiClient<DeleteAccountResponse>("/users/me", {
          method: "DELETE",
          headers: {
            "X-NyxID-Account-Deletion-Policy": ACCOUNT_DELETION_POLICY_VERSION,
          },
        }),
      );
      toast.success(
        result.status === "complete"
          ? "Account erasure complete"
          : "Account deletion accepted. Cleanup continues until verified.",
      );
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
      <GroupedList
        title="Two-factor authentication"
        footer={
          user?.mfa_enabled
            ? "Your account is protected with two-factor authentication."
            : "Add an extra layer of security to your account."
        }
      >
        <GroupedRow
          title="Two-factor authentication"
          accessory={
            <Switch
              checked={user?.mfa_enabled ?? false}
              onCheckedChange={(checked) => {
                if (checked) setMfaDialogOpen(true);
                else setDisableMfaDialogOpen(true);
              }}
              aria-label="Toggle two-factor authentication"
            />
          }
          description={user?.mfa_enabled ? "Enabled" : "Disabled"}
        />
      </GroupedList>
      <ApprovedSignInPolicy key={user?.id} />

      <MfaSetupDialog open={mfaDialogOpen} onOpenChange={setMfaDialogOpen} />

      <Dialog open={disableMfaDialogOpen} onOpenChange={handleDisableMfaClose}>
        <DialogContent>
          <GroupedDecisionDraft
            dirty={Boolean(disableMfaCode || disableMfaPassword)}
            pending={disableMfa.isPending}
            discard={() => {
              setDisableMfaCode("");
              setDisableMfaPassword("");
              setDisableMfaError(null);
            }}
          />
          <DialogHeader>
            <DialogTitle>Disable Two-Factor Authentication</DialogTitle>
            <DialogDescription>
              Enter a current authenticator or recovery code to confirm.{" "}
              {isPasswordUser && "Enter your password too. "}
              This will make your account less secure.
            </DialogDescription>
          </DialogHeader>
          <GroupedList
            footer={
              disableMfaError && (
                <p role="alert" className="text-destructive">
                  {disableMfaError}
                </p>
              )
            }
          >
            <GroupedRow className="block space-y-1">
              <Label htmlFor="disable-mfa-code">
                Authenticator or recovery code
              </Label>
              <Input
                id="disable-mfa-code"
                value={disableMfaCode}
                onChange={(e) => setDisableMfaCode(e.target.value)}
                placeholder="000000 or recovery code"
                autoComplete="one-time-code"
                inputMode="text"
                autoFocus
              />
            </GroupedRow>
            {isPasswordUser && (
              <GroupedRow className="block space-y-1">
                <Label htmlFor="disable-mfa-password">Password</Label>
                <Input
                  id="disable-mfa-password"
                  type="password"
                  autoComplete="current-password"
                  value={disableMfaPassword}
                  onChange={(e) => setDisableMfaPassword(e.target.value)}
                  placeholder="Enter your password"
                />
              </GroupedRow>
            )}
          </GroupedList>
          <DialogFooter className="bg-background">
            <GroupedDialogCancel
              variant="outline"
              disabled={disableMfa.isPending}
            >
              Cancel
            </GroupedDialogCancel>
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

      {user?.has_password === true && (
        <Form {...passwordForm}>
          <form
            aria-label="Change password"
            onSubmit={passwordForm.handleSubmit(handleChangePassword)}
            className="space-y-6"
          >
            <GroupedList
              title="Change password"
              footer={
                <>
                  <p>
                    Use 8–128 UTF-8 bytes. Existing sessions remain signed in.
                  </p>
                  {passwordForm.formState.errors.root && (
                    <p role="alert" className="text-destructive">
                      {passwordForm.formState.errors.root.message}
                    </p>
                  )}
                </>
              }
            >
              {(
                [
                  [
                    "currentPassword",
                    "Current Password",
                    showCurrentPw,
                    setShowCurrentPw,
                    "current-password",
                  ],
                  [
                    "newPassword",
                    "New Password",
                    showNewPw,
                    setShowNewPw,
                    "new-password",
                  ],
                  [
                    "confirmNewPassword",
                    "Confirm New Password",
                    showConfirmPw,
                    setShowConfirmPw,
                    "new-password",
                  ],
                ] as const
              ).map(([name, label, shown, setShown, autoComplete]) => (
                <FormField
                  key={name}
                  control={passwordForm.control}
                  name={name}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{label}</FormLabel>
                      <div className="relative">
                        <FormControl>
                          <Input
                            {...field}
                            type={shown ? "text" : "password"}
                            autoComplete={autoComplete}
                            className="pr-10"
                          />
                        </FormControl>
                        <button
                          type="button"
                          tabIndex={-1}
                          onClick={() => setShown((v) => !v)}
                          className="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
                          aria-label={shown ? "Hide password" : "Show password"}
                        >
                          {shown ? (
                            <Eye className="h-4 w-4" />
                          ) : (
                            <EyeOff className="h-4 w-4" />
                          )}
                        </button>
                      </div>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ))}
            </GroupedList>
            <GroupedFormActions
              label="Password"
              dirty={passwordForm.formState.isDirty}
              pending={passwordForm.formState.isSubmitting}
              save={passwordForm.handleSubmit(handleChangePassword)}
              discard={() => passwordForm.reset()}
              changes={() =>
                passwordForm.formState.isDirty
                  ? [
                      {
                        label: "Password",
                        before: "Current password",
                        after: "Changed",
                      },
                    ]
                  : []
              }
            />
          </form>
        </Form>
      )}

      <GroupedList
        title="Delete account"
        footer="Permanently delete your account and queue managed Machine workspace cleanup. Necessary security and financial records are retained."
      >
        <GroupedRow asChild interactive>
          <button
            type="button"
            className="text-destructive"
            onClick={() => setDeleteAccountOpen(true)}
          >
            <Trash2 aria-hidden className="size-[1.125em]" />
            Delete My Account
          </button>
        </GroupedRow>
      </GroupedList>

      <Dialog open={deleteAccountOpen} onOpenChange={handleDeleteAccountDialog}>
        <DialogContent>
          <GroupedDecisionDraft
            dirty={Boolean(deleteConfirmEmail)}
            pending={isDeletingAccount}
            discard={() => setDeleteConfirmEmail("")}
          />
          <DialogHeader>
            <DialogTitle>Delete Account</DialogTitle>
            <DialogDescription>
              Enter your email address to confirm deletion of your account and
              the Machine data described below.
            </DialogDescription>
          </DialogHeader>

          <AccountDeletionDisclosure />

          <GroupedList
            footer={
              deleteEmailError && (
                <p className="text-destructive">{deleteEmailError}</p>
              )
            }
          >
            <GroupedRow className="block space-y-1">
              <Label htmlFor="delete-account-email">Email</Label>
              <Input
                id="delete-account-email"
                value={deleteConfirmEmail}
                onChange={(e) => setDeleteConfirmEmail(e.target.value)}
                placeholder={user?.email ?? "your@email.com"}
                aria-invalid={deleteEmailError ? true : undefined}
                autoFocus
              />
            </GroupedRow>
          </GroupedList>

          <DialogFooter className="bg-background">
            <GroupedDialogCancel variant="outline" disabled={isDeletingAccount}>
              Cancel
            </GroupedDialogCancel>
            <Button
              variant="destructive"
              isLoading={isDeletingAccount}
              disabled={!deleteEmailMatches || isDeletingAccount}
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
      <p className="px-4 text-13 text-text-tertiary">
        Install NyxID as an MCP (Model Context Protocol) server in your client
        of choice.
      </p>
      <GroupedList
        title="Install to Cursor"
        footer="One-click install via Cursor's deeplink protocol. Cursor will open and prompt you to confirm the MCP server installation."
      >
        <GroupedRow asChild interactive>
          <button
            type="button"
            onClick={() => void openExternal(cursorDeeplink)}
          >
            <ExternalLink aria-hidden className="size-[1.125em]" />
            Install to Cursor
          </button>
        </GroupedRow>
        <McpCode
          title="Or copy manually · .cursor/mcp.json"
          text={cursorConfig}
          label="Cursor config"
        />
      </GroupedList>
      <GroupedList
        title="Install to Claude Code"
        footer="Run this command in your terminal to add NyxID as an MCP server in Claude Code."
      >
        <McpCode
          title="Terminal command"
          text={claudeCommand}
          label="CLI command"
        />
        <McpCode
          title="Or add manually · .claude/settings.json or .mcp.json"
          text={claudeConfig}
          label="Claude Code config"
        />
      </GroupedList>
      <GroupedList
        title="Install to Codex"
        footer="Run this command in your terminal to add NyxID as an MCP server in Codex CLI."
      >
        <McpCode
          title="Terminal command"
          text={codexCommand}
          label="CLI command"
        />
        <McpCode
          title="Or add manually · ~/.codex/config.toml"
          text={codexConfig}
          label="Codex config"
        />
      </GroupedList>
      <GroupedList
        title="How it works"
        footer="When your MCP client connects for the first time, NyxID will open an OAuth flow in your browser to authenticate. Once authenticated, the proxy exposes tools from all your connected services. Tool calls are forwarded to each service's API with your credentials injected automatically."
      />
    </div>
  );
}

function McpCode({
  title,
  text,
  label,
}: {
  readonly title: string;
  readonly text: string;
  readonly label: string;
}) {
  return (
    <GroupedRow className="relative block space-y-1 pr-12">
      <p className="text-13 text-text-tertiary">{title}</p>
      <pre className="font-mono type-code whitespace-pre-wrap break-all">
        {text}
      </pre>
      <CopyInlineButton text={text} label={label} />
    </GroupedRow>
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

  // The backend now stamps `is_current=true` on the row this request came
  // from, so the dialog copy + Current pill can vary per session. Revoking
  // the current session still works — the next request 401s and the auth
  // interceptor bounces to /login (GitHub-style trust parity). The dialog
  // calls that out explicitly so it isn't a surprise.
  const [revokeTarget, setRevokeTarget] = useState<Session | null>(null);
  const revokeMutation = useRevokeSession();

  async function handleRevoke() {
    if (!revokeTarget) return;
    try {
      await revokeMutation.mutateAsync(revokeTarget.id);
      toast.success(
        revokeTarget.is_current
          ? "Session revoked. Signing you out…"
          : "Session revoked",
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to revoke session",
      );
    } finally {
      setRevokeTarget(null);
    }
  }

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  return (
    <div className="space-y-6">
      <GroupedList
        title="Active sessions"
        footer="Manage your active sessions across devices."
      >
        <GroupedRow asChild interactive>
          <Link to="/login/code">Generate terminal login code</Link>
        </GroupedRow>
        {!sessions?.length ? (
          <GroupedRow
            title="No Active Sessions"
            description="Your active sessions across devices will appear here."
          />
        ) : (
          sessions.map((session) => {
            const deviceLabel = humanizeUserAgent(session.user_agent);
            return (
              <GroupedRow
                key={session.id}
                leading={getDeviceIcon(session.user_agent)}
                title={
                  <span className="flex flex-wrap items-center gap-2">
                    {deviceLabel}
                    {session.is_current && (
                      <Badge variant="success">Current</Badge>
                    )}
                  </span>
                }
                description={
                  <div className="space-y-1">
                    <p>IP Address: {humanizeIpAddress(session.ip_address)}</p>
                    <p>Signed in: {formatDate(session.created_at)}</p>
                    <p>Expires: {formatDate(session.expires_at)}</p>
                  </div>
                }
                accessory={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="text-destructive"
                    onClick={() => setRevokeTarget(session)}
                    aria-label={
                      session.is_current
                        ? "Revoke current session and sign out"
                        : `Revoke session on ${deviceLabel}`
                    }
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                }
              />
            );
          })
        )}
      </GroupedList>

      <Dialog
        open={revokeTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRevokeTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {revokeTarget?.is_current
                ? "Sign out of this session?"
                : "Revoke session?"}
            </DialogTitle>
            <DialogDescription>
              {revokeTarget?.is_current ? (
                <>
                  This is the session you&apos;re currently using on{" "}
                  <span className="font-medium">
                    {humanizeUserAgent(revokeTarget?.user_agent)}
                  </span>
                  . Revoking it will sign you out immediately and you&apos;ll
                  need to sign in again.
                </>
              ) : (
                <>
                  This will immediately invalidate the session on{" "}
                  <span className="font-medium">
                    {humanizeUserAgent(revokeTarget?.user_agent)}
                  </span>
                  . The person using it will be signed out next time they load a
                  page.
                </>
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="bg-background">
            <Button variant="outline" onClick={() => setRevokeTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleRevoke()}
              isLoading={revokeMutation.isPending}
            >
              {revokeTarget?.is_current ? "Revoke and sign out" : "Revoke"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function PrivacyTab() {
  const consentEnabled = useConsentStore((s) => s.enabled);
  const persistenceError = useConsentStore((s) => s.persistenceError);
  const setConsent = useConsentStore((s) => s.setConsent);
  const accountId = useAuthStore((s) => s.user?.id);
  const [saving, setSaving] = useState(false);
  const permission = useQuery({
    queryKey: ["analytics-consent", accountId],
    queryFn: () => readAnalyticsConsent(accountId!),
    enabled: Boolean(accountId),
    staleTime: 0,
  });
  async function handleToggle(next: boolean) {
    if (!accountId) return;
    setSaving(true);
    try {
      await saveAnalyticsConsent(accountId, next);
      await permission.refetch();
    } catch {
      toast.error(
        "Could not save account analytics permission. This browser remains off; retry to withdraw across devices.",
      );
    } finally {
      setSaving(false);
    }
  }

  return (
    <GroupedList
      title="Usage analytics"
      footer={
        <>
          <p>
            Optional usage events are linked to your account. Account permission
            applies across devices. Each device must also opt in locally; Do Not
            Track always disables collection. Security audit and billing records
            have separate retention rules.
          </p>
          {persistenceError && (
            <div role="alert" className="space-y-2">
              <p>
                This browser's choice could not be saved. Analytics is off in
                this session; retry before closing this page.
              </p>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setConsent(false)}
              >
                Retry saving browser choice
              </Button>
            </div>
          )}
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
        </>
      }
    >
      <GroupedRow
        title="Allow account usage analytics"
        description={
          <span role="status">
            {permission.isFetching
              ? "Loading account permission…"
              : permission.isError
                ? "Account permission unavailable"
                : permission.data?.enabled
                  ? "Account permission enabled"
                  : "Account permission disabled"}
          </span>
        }
        accessory={
          <>
            {permission.isError && (
              <Button
                variant="ghost"
                size="sm"
                isLoading={permission.isFetching}
                onClick={() => void permission.refetch()}
              >
                Retry
              </Button>
            )}
            <Switch
              checked={permission.data?.enabled === true}
              disabled={
                saving ||
                !accountId ||
                !permission.data ||
                permission.isError ||
                permission.isFetching
              }
              onCheckedChange={(next) => void handleToggle(next)}
              aria-label="Allow account usage analytics"
            />
          </>
        }
      />
      <GroupedRow
        title="Allow on this browser"
        accessory={
          <Switch
            checked={consentEnabled}
            disabled={
              !consentEnabled &&
              (permission.data?.enabled !== true ||
                permission.isError ||
                permission.isFetching)
            }
            onCheckedChange={(next) => {
              setConsent(next);
              if (!next) disableTelemetry();
            }}
            aria-label="Allow usage analytics on this browser"
          />
        }
      />
    </GroupedList>
  );
}
