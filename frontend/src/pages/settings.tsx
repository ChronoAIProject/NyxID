import { useState } from "react"
import { useForm } from "react-hook-form"
import { zodResolver } from "@hookform/resolvers/zod"
import { useQuery } from "@tanstack/react-query"
import { useAuthStore } from "@/stores/auth-store"
import { useUser, useMfaDisable } from "@/hooks/use-auth"
import { api, ApiError } from "@/lib/api-client"
import type { User, Session } from "@/types/api"
import {
  changePasswordSchema,
  type ChangePasswordFormData,
} from "@/schemas/auth"
import { formatDate } from "@/lib/utils"
import { MfaSetupDialog } from "@/components/auth/mfa-setup-dialog"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Separator } from "@/components/ui/separator"
import { Skeleton } from "@/components/ui/skeleton"
import { Switch } from "@/components/ui/switch"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import {
  ShieldCheck,
  ShieldOff,
  Monitor,
  Smartphone,
  Globe,
} from "lucide-react"
import { toast } from "sonner"

export function SettingsPage() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-3xl font-bold tracking-tight">Settings</h2>
        <p className="text-muted-foreground">
          Manage your account settings and preferences.
        </p>
      </div>

      <Tabs defaultValue="profile" className="space-y-6">
        <TabsList>
          <TabsTrigger value="profile">Profile</TabsTrigger>
          <TabsTrigger value="security">Security</TabsTrigger>
          <TabsTrigger value="sessions">Sessions</TabsTrigger>
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
      </Tabs>
    </div>
  )
}

function ProfileTab() {
  const { data: user, isLoading } = useUser()
  const [name, setName] = useState("")
  const [saving, setSaving] = useState(false)
  const setUser = useAuthStore((s) => s.setUser)

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />
  }

  const displayName = name || user?.name || ""

  async function handleSave() {
    setSaving(true)
    try {
      const updated = await api.put<User>("/users/me", {
        display_name: displayName,
      })
      setUser(updated)
      toast.success("Profile updated successfully")
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message)
      } else {
        toast.error("Failed to update profile")
      }
    } finally {
      setSaving(false)
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Profile</CardTitle>
        <CardDescription>
          Update your personal information.
        </CardDescription>
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
  )
}

function SecurityTab() {
  const user = useAuthStore((s) => s.user)
  const [mfaDialogOpen, setMfaDialogOpen] = useState(false)
  const [disableMfaDialogOpen, setDisableMfaDialogOpen] = useState(false)
  const [disableMfaPassword, setDisableMfaPassword] = useState("")
  const [disableMfaError, setDisableMfaError] = useState<string | null>(null)
  const disableMfa = useMfaDisable()

  const passwordForm = useForm<ChangePasswordFormData>({
    resolver: zodResolver(changePasswordSchema),
    defaultValues: {
      currentPassword: "",
      newPassword: "",
      confirmNewPassword: "",
    },
  })

  async function handleChangePassword(data: ChangePasswordFormData) {
    try {
      await api.post<void>("/auth/password/change", {
        current_password: data.currentPassword,
        new_password: data.newPassword,
      })
      toast.success("Password changed successfully")
      passwordForm.reset()
    } catch (error) {
      if (error instanceof ApiError) {
        passwordForm.setError("root", { message: error.message })
      } else {
        toast.error("Failed to change password")
      }
    }
  }

  async function handleDisableMfa() {
    if (!disableMfaPassword) {
      setDisableMfaError("Password is required")
      return
    }
    try {
      await disableMfa.mutateAsync(disableMfaPassword)
      toast.success("MFA disabled")
      setDisableMfaDialogOpen(false)
      setDisableMfaPassword("")
      setDisableMfaError(null)
    } catch (error) {
      if (error instanceof ApiError) {
        setDisableMfaError(error.message)
      } else {
        setDisableMfaError("Failed to disable MFA")
      }
    }
  }

  function handleDisableMfaClose() {
    setDisableMfaDialogOpen(false)
    setDisableMfaPassword("")
    setDisableMfaError(null)
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            {user?.mfa_enabled ? (
              <ShieldCheck className="h-5 w-5 text-emerald-400" aria-hidden="true" />
            ) : (
              <ShieldOff className="h-5 w-5 text-muted-foreground" aria-hidden="true" />
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
                    setMfaDialogOpen(true)
                  } else {
                    setDisableMfaDialogOpen(true)
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
    </div>
  )
}

function getDeviceIcon(userAgent: string) {
  const ua = userAgent.toLowerCase()
  if (
    ua.includes("mobile") ||
    ua.includes("android") ||
    ua.includes("iphone")
  ) {
    return <Smartphone className="h-4 w-4" aria-hidden="true" />
  }
  if (
    ua.includes("mozilla") ||
    ua.includes("chrome") ||
    ua.includes("safari")
  ) {
    return <Monitor className="h-4 w-4" aria-hidden="true" />
  }
  return <Globe className="h-4 w-4" aria-hidden="true" />
}

function SessionsTab() {
  const { data: sessions, isLoading } = useQuery({
    queryKey: ["sessions"],
    queryFn: async (): Promise<readonly Session[]> => {
      return api.get<readonly Session[]>("/sessions")
    },
  })

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />
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
  )
}
