import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
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
import { LanguageSettings } from "@/components/settings/language-settings";
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
import {
  Eye,
  EyeOff,
  Pencil,
  Trash2,
  ExternalLink,
  Copy,
  Check,
} from "lucide-react";
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
  useUiTranslation();
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
  useUiTranslation();
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
        aria-label={translateUi("Account settings")}
        className={
          presentation === "panel"
            ? "flex min-h-0 flex-1 flex-col"
            : "space-y-8"
        }
      >
        {presentation === "page" && (
          <PageHeader
            title={translateUi("Account Settings")}
            description={translateUi("Manage your account settings and preferences.")}
          />
        )}
        <SegmentedControl
          value={currentTab}
          onValueChange={handleTabChange}
          className="flex min-h-0 flex-1 flex-col gap-6"
        >
          <CompactChrome>
            <SegmentedControlList aria-label={translateUi("Account settings")}>
              {SETTINGS_TABS.map((tab) => (
                <SegmentedControlTrigger key={tab} value={tab}>
                  {tab === "mcp"
                    ? translateUi("MCP")
                    : translateUi(tab.charAt(0).toUpperCase() + tab.slice(1))}
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
                    ? translateUi("MCP")
                    : translateUi(tab.charAt(0).toUpperCase() + tab.slice(1))}
                </h2>
                <FormViewport className="flex min-h-0 flex-1 flex-col">
                  <CompactScroll
                    active={currentTab === tab}
                    className="assistant-scrollbar flex-1 space-y-6 pb-6"
                  >
                    {tab === "profile" ? (
                      <>
                        <ProfileTab />
                        <LanguageSettings />
                      </>
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
  useUiTranslation();
  const { data: user, isLoading } = useUser();
  if (isLoading || !user) return <Skeleton className="h-32 w-full" />;
  return <ProfileForm key={user.id} user={user} />;
}

function ProfileForm({ user }: { readonly user: User }) {
  useUiTranslation();
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
      toast.success(translateUi("Profile updated successfully"));
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
        aria-label={translateUi("Profile")}
        onSubmit={form.handleSubmit(save)}
        className="space-y-6"
      >
        <GroupedList
          title={translateUi("Profile")}
          footer={
            <>
              <p>{translateUi("Update your personal information.")}</p>
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
              <FormItem>
                <FormLabel>{translateUi("Name")}</FormLabel>
                <div className="relative">
                  <FormControl>
                    <Input
                      {...field}
                      className="pr-8"
                      placeholder={translateUi("Your name")}
                    />
                  </FormControl>
                  <Pencil
                    aria-hidden="true"
                    className="pointer-events-none absolute right-0 top-1/2 size-4 -translate-y-1/2 text-text-tertiary"
                  />
                </div>
                <FormMessage />
              </FormItem>
            )}
          />
          <GroupedRow className="block space-y-0.5">
            <div className="flex items-center justify-between gap-3">
              <Label htmlFor="profile-email">{translateUi("Email")}</Label>
              <Badge variant={user.email_verified ? "success" : "warning"}>
                {user.email_verified ? translateUi("Verified") : translateUi("Not verified")}
              </Badge>
            </div>
            <Input
              id="profile-email"
              value={user.email}
              disabled
              aria-readonly="true"
              title={user.email}
              className="truncate text-text-tertiary disabled:opacity-100"
            />
          </GroupedRow>
          {!user.email_verified && (
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() =>
                  toast.info(
                    translateUi("Email resend is not yet available on this server."),
                  )
                }
              >{translateUi("Resend verification email")}</button>
            </GroupedRow>
          )}
        </GroupedList>
        <GroupedFormActions
          label={translateUi("Profile")}
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
  useUiTranslation();
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
      toast.success(translateUi("Password changed successfully"));
      passwordForm.reset();
    } catch (error) {
      if (error instanceof ApiError) {
        passwordForm.setError("root", { message: error.message });
      } else {
        toast.error(translateErrorMessage("Failed to change password"));
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
    setDisableMfaError(null);
    try {
      await disableMfa.mutateAsync({
        code: disableMfaCode.trim(),
        ...(isPasswordUser ? { password: disableMfaPassword } : {}),
      });
      toast.success(translateUi("MFA disabled"));
      setDisableMfaDialogOpen(false);
      setDisableMfaCode("");
      setDisableMfaPassword("");
      setDisableMfaError(null);
    } catch (error) {
      if (error instanceof ApiError) {
        setDisableMfaError(error.message);
      } else {
        setDisableMfaError(translateUi("Failed to disable MFA"));
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
      toast.error(translateErrorMessage("Please enter your email to confirm"));
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
        translateUi(result.status === "complete"
          ? "Account erasure complete"
          : "Account deletion accepted. Cleanup continues until verified."),
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
        toast.error(translateErrorMessage(error.message));
      } else {
        toast.error(translateErrorMessage("Failed to delete account"));
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
        title={translateUi("Two-factor authentication")}
        footer={
          user?.mfa_enabled
            ? translateUi("Your account is protected with two-factor authentication.")
            : translateUi("Add an extra layer of security to your account.")
        }
      >
        <GroupedRow
          title={translateUi("Two-factor authentication")}
          accessory={
            <Switch
              checked={user?.mfa_enabled ?? false}
              disabled={disableMfa.isPending}
              onCheckedChange={(checked) => {
                if (checked) setMfaDialogOpen(true);
                else setDisableMfaDialogOpen(true);
              }}
              aria-label={translateUi("Toggle two-factor authentication")}
            />
          }
          description={user?.mfa_enabled ? translateUi("Enabled") : translateUi("Disabled")}
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
            <DialogTitle>{translateUi("Disable Two-Factor Authentication")}</DialogTitle>
            <DialogDescription>{translateUi("Enter a current authenticator or recovery code to confirm.")}{" "}
              {isPasswordUser && "Enter your password too. "}{translateUi("This will make your account less secure.")}</DialogDescription>
          </DialogHeader>
          <GroupedList
            aria-busy={disableMfa.isPending}
            footer={
              disableMfaError && (
                <p role="alert" className="text-destructive">
                  {translateUi(disableMfaError)}
                </p>
              )
            }
          >
            <GroupedRow className="block space-y-1">
              <Label htmlFor="disable-mfa-code">{translateUi("Authenticator or recovery code")}</Label>
              <Input
                id="disable-mfa-code"
                disabled={disableMfa.isPending}
                value={disableMfaCode}
                onChange={(e) => setDisableMfaCode(e.target.value)}
                placeholder={translateUi("000000 or recovery code")}
                autoComplete="one-time-code"
                inputMode="text"
                autoFocus
              />
            </GroupedRow>
            {isPasswordUser && (
              <GroupedRow className="block space-y-1">
                <Label htmlFor="disable-mfa-password">{translateUi("Password")}</Label>
                <Input
                  id="disable-mfa-password"
                  disabled={disableMfa.isPending}
                  type="password"
                  autoComplete="current-password"
                  value={disableMfaPassword}
                  onChange={(e) => setDisableMfaPassword(e.target.value)}
                  placeholder={translateUi("Enter your password")}
                />
              </GroupedRow>
            )}
          </GroupedList>
          {disableMfa.isPending && (
            <p role="status" className="px-4 text-13 text-text-tertiary">{translateUi("Disabling two-factor authentication…")}</p>
          )}
          <DialogFooter className="bg-background">
            <GroupedDialogCancel
              variant="outline"
              disabled={disableMfa.isPending}
            >{translateUi("Cancel")}</GroupedDialogCancel>
            <Button
              variant="destructive"
              onClick={() => void handleDisableMfa()}
              isLoading={disableMfa.isPending}
            >{translateUi("Disable MFA")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {user?.has_password === true && (
        <Form {...passwordForm}>
          <form
            aria-label={translateUi("Change password")}
            onSubmit={passwordForm.handleSubmit(handleChangePassword)}
            className="space-y-6"
          >
            <GroupedList
              title={translateUi("Change password")}
              footer={
                <>
                  <p>{translateUi("Use 8–128 UTF-8 bytes. Existing sessions remain signed in.")}</p>
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
                            placeholder={
                              name === "currentPassword"
                                ? translateUi("Enter current password")
                                : name === "newPassword"
                                  ? translateUi("Enter new password")
                                  : translateUi("Re-enter new password")
                            }
                            className="pr-10"
                          />
                        </FormControl>
                        <button
                          type="button"
                          onClick={() => setShown((v) => !v)}
                          className="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
                          aria-label={`${shown ? "Hide" : "Show"} ${label.toLowerCase()}`}
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
              label={translateUi("Password")}
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
        title={translateUi("Delete account")}
        footer={translateUi("Permanently delete your account and queue managed Machine workspace cleanup. Necessary security and financial records are retained.")}
      >
        <GroupedRow asChild interactive>
          <button
            type="button"
            className="text-destructive"
            onClick={() => setDeleteAccountOpen(true)}
          >
            <Trash2 aria-hidden className="size-[1.125em]" />{translateUi("Delete My Account")}</button>
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
            <DialogTitle>{translateUi("Delete Account")}</DialogTitle>
            <DialogDescription>{translateUi("Enter your email address to confirm deletion of your account and the Machine data described below.")}</DialogDescription>
          </DialogHeader>

          <AccountDeletionDisclosure />

          <GroupedList
            footer={
              deleteEmailError && (
                <p className="text-destructive">{translateUi(deleteEmailError)}</p>
              )
            }
          >
            <GroupedRow className="block space-y-1">
              <Label htmlFor="delete-account-email">{translateUi("Email")}</Label>
              <Input
                id="delete-account-email"
                value={deleteConfirmEmail}
                onChange={(e) => setDeleteConfirmEmail(e.target.value)}
                placeholder={user?.email ?? translateUi("your@email.com")}
                aria-invalid={deleteEmailError ? true : undefined}
                autoFocus
              />
            </GroupedRow>
          </GroupedList>

          <DialogFooter className="bg-background">
            <GroupedDialogCancel variant="outline" disabled={isDeletingAccount}>{translateUi("Cancel")}</GroupedDialogCancel>
            <Button
              variant="destructive"
              isLoading={isDeletingAccount}
              disabled={!deleteEmailMatches || isDeletingAccount}
              onClick={() => void handleDeleteAccount()}
            >{translateUi("Permanently Delete")}</Button>
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
  useUiTranslation();
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await copyToClipboard(text);
      setCopied(true);
      toast.success(translateUi(`${label} copied to clipboard`));
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error(translateErrorMessage("Failed to copy to clipboard"));
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
      <span className="sr-only">{translateUi("Copy ")}{label}</span>
    </Button>
  );
}

function McpTab() {
  useUiTranslation();
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
      <p className="px-4 text-13 text-text-tertiary">{translateUi("Install NyxID as an MCP (Model Context Protocol) server in your client of choice.")}</p>
      <GroupedList
        title={translateUi("Install to Cursor")}
        footer={translateUi("One-click install via Cursor's deeplink protocol. Cursor will open and prompt you to confirm the MCP server installation.")}
      >
        <GroupedRow asChild interactive>
          <button
            type="button"
            onClick={() => void openExternal(cursorDeeplink)}
          >
            <ExternalLink aria-hidden className="size-[1.125em]" />{translateUi("Install to Cursor")}</button>
        </GroupedRow>
        <McpCode
          title={translateUi("Or copy manually · .cursor/mcp.json")}
          text={cursorConfig}
          label={translateUi("Cursor config")}
        />
      </GroupedList>
      <GroupedList
        title={translateUi("Install to Claude Code")}
        footer={translateUi("Run this command in your terminal to add NyxID as an MCP server in Claude Code.")}
      >
        <McpCode
          title={translateUi("Terminal command")}
          text={claudeCommand}
          label={translateUi("CLI command")}
        />
        <McpCode
          title={translateUi("Or add manually · .claude/settings.json or .mcp.json")}
          text={claudeConfig}
          label={translateUi("Claude Code config")}
        />
      </GroupedList>
      <GroupedList
        title={translateUi("Install to Codex")}
        footer={translateUi("Run this command in your terminal to add NyxID as an MCP server in Codex CLI.")}
      >
        <McpCode
          title={translateUi("Terminal command")}
          text={codexCommand}
          label={translateUi("CLI command")}
        />
        <McpCode
          title={translateUi("Or add manually · ~/.codex/config.toml")}
          text={codexConfig}
          label={translateUi("Codex config")}
        />
      </GroupedList>
      <GroupedList
        title={translateUi("How it works")}
        footer={translateUi("When your MCP client connects for the first time, NyxID will open an OAuth flow in your browser to authenticate. Once authenticated, the proxy exposes tools from all your connected services. Tool calls are forwarded to each service's API with your credentials injected automatically.")}
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
  useUiTranslation();
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
  useUiTranslation();
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
        translateUi(revokeTarget.is_current
          ? "Session revoked. Signing you out…"
          : "Session revoked"),
      );
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to revoke session"),
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
        title={translateUi("Active sessions")}
        footer={translateUi("Manage your active sessions across devices.")}
      >
        <GroupedRow asChild interactive>
          <Link to="/login/code">{translateUi("Generate terminal login code")}</Link>
        </GroupedRow>
        {!sessions?.length ? (
          <GroupedRow
            title={translateUi("No Active Sessions")}
            description={translateUi("Your active sessions across devices will appear here.")}
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
                      <Badge variant="success">{translateUi("Current")}</Badge>
                    )}
                  </span>
                }
                description={
                  <div className="space-y-1">
                    <p>{translateUi("IP Address: ")}{humanizeIpAddress(session.ip_address)}</p>
                    <p>{translateUi("Signed in: ")}{formatDate(session.created_at)}</p>
                    <p>{translateUi("Expires: ")}{formatDate(session.expires_at)}</p>
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
                        ? translateUi("Revoke current session and sign out")
                        : translateUi("Revoke session on {{value1}}", { value1: deviceLabel })
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
                ? translateUi("Sign out of this session?")
                : translateUi("Revoke session?")}
            </DialogTitle>
            <DialogDescription>
              {revokeTarget?.is_current ? (
                <>{translateUi("This is the session you're currently using on")}{" "}
                  <span className="font-medium">
                    {humanizeUserAgent(revokeTarget?.user_agent)}
                  </span>
                  {translateUi(". Revoking it will sign you out immediately and you'll need to sign in again. ")}</>
              ) : (
                <>{translateUi("This will immediately invalidate the session on")}{" "}
                  <span className="font-medium">
                    {humanizeUserAgent(revokeTarget?.user_agent)}
                  </span>
                  {translateUi(". The person using it will be signed out next time they load a page. ")}</>
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="bg-background">
            <Button variant="outline" onClick={() => setRevokeTarget(null)}>{translateUi("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleRevoke()}
              isLoading={revokeMutation.isPending}
            >
              {revokeTarget?.is_current ? translateUi("Revoke and sign out") : translateUi("Revoke")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function PrivacyTab() {
  useUiTranslation();
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
        translateErrorMessage("Could not save account analytics permission. This browser remains off; retry to withdraw across devices."),
      );
    } finally {
      setSaving(false);
    }
  }

  return (
    <GroupedList
      title={translateUi("Usage analytics")}
      footer={
        <>
          <p>{translateUi("Optional usage events are linked to your account. Account permission applies across devices. Each device must also opt in locally; Do Not Track always disables collection. Security audit and billing records have separate retention rules.")}</p>
          {persistenceError && (
            <div role="alert" className="space-y-2">
              <p>{translateUi("This browser's choice could not be saved. Analytics is off in this session; retry before closing this page.")}</p>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setConsent(false)}
              >{translateUi("Retry saving browser choice")}</Button>
            </div>
          )}
          <p>{translateUi("For the full disclosure of what we collect, how it's stored, and retention windows, see the")}{" "}
            <a
              href="/privacy"
              className="underline underline-offset-2 hover:text-foreground"
            >{translateUi("privacy policy")}</a>
            .
          </p>
        </>
      }
    >
      <GroupedRow
        title={translateUi("Allow account usage analytics")}
        description={
          <span role="status">
            {permission.isFetching
              ? translateUi("Loading account permission…")
              : permission.isError
                ? translateUi("Account permission unavailable")
                : permission.data?.enabled
                  ? translateUi("Account permission enabled")
                  : translateUi("Account permission disabled")}
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
              >{translateUi("Retry")}</Button>
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
              aria-label={translateUi("Allow account usage analytics")}
            />
          </>
        }
      />
      <GroupedRow
        title={translateUi("Allow on this browser")}
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
            aria-label={translateUi("Allow usage analytics on this browser")}
          />
        }
      />
    </GroupedList>
  );
}
