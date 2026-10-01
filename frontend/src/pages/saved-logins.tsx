import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { zodResolver } from "@hookform/resolvers/zod";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  Form,
  FormField,
  FormItem,
  FormControl,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import {
  useSavedLogins,
  saveLogin,
  useDeleteSavedLogin,
} from "@/hooks/use-saved-logins";
import {
  savedLoginSchema,
  SAVED_LOGIN_RESIDUAL,
  type SavedLogin,
  type SavedLoginInput,
} from "@/schemas/saved-logins";

export function SavedLoginsPage() {
  const [owner, setOwner] = useState<string | null>(null);
  const logins = useSavedLogins(owner);
  const remove = useDeleteSavedLogin();
  const [editing, setEditing] = useState<SavedLogin | "new">();
  const [deleting, setDeleting] = useState<SavedLogin>();
  return (
    <div className="space-y-6">
      <PageHeader
        title="Saved logins"
        description="Let NyxBot sign in to approved websites. Specialists need an explicit grant. Never put a password in chat."
        actions={
          <AddCtaButton label="Add login" onClick={() => setEditing("new")} />
        }
      />
      <div className="max-w-xs">
        <OrgScopeSelect value={owner} onChange={setOwner} />
      </div>
      <p className="rounded-lg border border-border p-4 text-[12px] text-muted-foreground">
        {SAVED_LOGIN_RESIDUAL}
      </p>
      {logins.isLoading ? (
        <p className="text-[12px]">Loading saved logins…</p>
      ) : null}
      {logins.error || remove.error ? (
        <p role="alert" className="text-[12px] text-destructive">
          {(logins.error ?? remove.error)?.message}
        </p>
      ) : null}
      {!logins.isLoading && !logins.data?.length ? (
        <p className="text-[12px] text-muted-foreground">
          No saved logins in this account.
        </p>
      ) : null}
      <div className="grid gap-4 sm:grid-cols-2">
        {logins.data?.map((login) => (
          <SavedLoginCard
            key={login.id}
            login={login}
            onEdit={setEditing}
            onDelete={setDeleting}
          />
        ))}
      </div>
      {editing ? (
        <LoginEditor
          key={editing === "new" ? "new" : editing.id}
          login={editing === "new" ? undefined : editing}
          owner={owner}
          onClose={() => setEditing(undefined)}
        />
      ) : null}
      <Dialog
        open={Boolean(deleting)}
        onOpenChange={(open) => {
          if (!open) setDeleting(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {deleting?.label}?</DialogTitle>
            <DialogDescription>
              This removes the saved login and its specialist grants.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleting(undefined)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              isLoading={remove.isPending}
              onClick={() => {
                if (deleting)
                  void remove
                    .mutateAsync(deleting.id)
                    .then(() => setDeleting(undefined))
                    .catch(() => undefined);
              }}
            >
              Delete login
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
function SavedLoginCard({
  login,
  onEdit,
  onDelete,
}: {
  readonly login: SavedLogin;
  readonly onEdit: (login: SavedLogin) => void;
  readonly onDelete: (login: SavedLogin) => void;
}) {
  return (
    <article className="space-y-3 rounded-xl border border-border bg-card p-4">
      <h2 className="font-display text-[15px] font-medium">{login.label}</h2>
      <p className="break-all text-[12px] text-muted-foreground">
        {login.allowed_origins.join(", ")}
      </p>
      <p className="text-[12px]">Username {login.username_hint}</p>
      <div className="flex flex-wrap gap-2">
        {login.has_password ? (
          <Badge variant="secondary">Password set</Badge>
        ) : null}
        {login.has_totp ? (
          <Badge variant="secondary">One-time codes</Badge>
        ) : null}
        {login.confirm_each_sign_in ? (
          <Badge variant="info">Confirm each sign-in</Badge>
        ) : null}
      </div>
      <div className="flex gap-2">
        <Button onClick={() => onEdit(login)}>Replace</Button>
        <Button variant="destructive" onClick={() => onDelete(login)}>
          Delete
        </Button>
      </div>
    </article>
  );
}

function LoginEditor({
  login,
  owner,
  onClose,
}: {
  readonly login?: SavedLogin;
  readonly owner: string | null;
  readonly onClose: () => void;
}) {
  const query = useQueryClient();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const form = useAppForm<SavedLoginInput>({
    resolver: zodResolver(savedLoginSchema),
    defaultValues: {
      label: login?.label ?? "",
      allowed_origins: login?.allowed_origins ?? [""],
      username: "",
      password: "",
      totp_secret: "",
      confirm_each_sign_in: login?.confirm_each_sign_in ?? false,
    },
  });
  async function submit(values: SavedLoginInput) {
    setSaving(true);
    setError(undefined);
    try {
      await saveLogin(values, login?.id, owner);
      form.reset();
      await query.invalidateQueries({ queryKey: ["saved-logins"] });
      onClose();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not save login");
    } finally {
      values.username = "";
      values.password = "";
      values.totp_secret = "";
      setSaving(false);
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !saving) {
          form.reset();
          onClose();
        }
      }}
    >
      <DialogContent
        className="ph-no-capture"
        data-private="true"
        data-ph-no-capture
      >
        <DialogHeader>
          <DialogTitle>
            {login ? "Replace saved login" : "Add saved login"}
          </DialogTitle>
          <DialogDescription>
            Secrets are write-only.{" "}
            {login
              ? "Re-enter every value to replace this login atomically."
              : "Enter them here, never in chat."}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(submit)}
            className="space-y-4"
            autoComplete="off"
          >
            <FormField
              control={form.control}
              name="label"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Label</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="allowed_origins"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Allowed HTTPS origins (one per line)</FormLabel>
                  <FormControl>
                    <Textarea
                      className="min-h-20"
                      value={field.value.join("\n")}
                      onChange={(event) =>
                        field.onChange(event.target.value.split("\n"))
                      }
                    />
                  </FormControl>
                  <FormMessage />
                  {field.value.map((_, index) => {
                    const message =
                      form.formState.errors.allowed_origins?.[index]?.message;
                    return message ? (
                      <p
                        key={index}
                        role="alert"
                        className="text-[12px] text-destructive"
                      >
                        Line {index + 1}: {message}
                      </p>
                    ) : null;
                  })}
                </FormItem>
              )}
            />
            {(["username", "password", "totp_secret"] as const).map((name) => (
              <FormField
                key={name}
                control={form.control}
                name={name}
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      {name === "username"
                        ? "Username"
                        : name === "password"
                          ? "Password (optional)"
                          : "TOTP secret or otpauth URI (optional)"}
                    </FormLabel>
                    <FormControl>
                      <Input
                        {...field}
                        type={name === "username" ? "text" : "password"}
                        autoComplete={
                          name === "username" ? "off" : "new-password"
                        }
                        spellCheck={false}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            ))}
            <FormField
              control={form.control}
              name="confirm_each_sign_in"
              render={({ field }) => (
                <FormItem className="flex items-center justify-between">
                  <FormLabel>Confirm each sign-in</FormLabel>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                  </FormControl>
                </FormItem>
              )}
            />
            {error ? (
              <p role="alert" className="text-[12px] text-destructive">
                {error}
              </p>
            ) : null}
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                disabled={saving}
                onClick={() => {
                  form.reset();
                  onClose();
                }}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                isLoading={saving}
                disabled={
                  !form.formState.isDirty ||
                  !form.watch("username") ||
                  !form.watch("label")
                }
              >
                Save login
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
