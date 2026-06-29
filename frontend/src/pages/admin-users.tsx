import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAdminUsers, useCreateUser } from "@/hooks/use-admin";
import { createUserSchema, type CreateUserFormData } from "@/schemas/admin";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { resolvePlatformRole, canAdminWrite } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Search,
  ChevronLeft,
  ChevronRight,
  Eye,
  EyeOff,
} from "lucide-react";
import { BiometricIdentityIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";

const PER_PAGE = 20;

export function AdminUsersPage() {
  const navigate = useNavigate();
  const [page, setPage] = useState(1);
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);

  const currentUser = useAuthStore((s) => s.user);
  const canWrite = canAdminWrite(currentUser);

  const { data, isLoading, error } = useAdminUsers(page, PER_PAGE, search);
  const createMutation = useCreateUser();

  const users = data?.users ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / PER_PAGE));

  const createForm = useForm<CreateUserFormData>({
    resolver: zodResolver(createUserSchema),
    defaultValues: {
      email: "",
      password: "",
      display_name: "",
      role: "user",
    },
  });

  function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    setSearch(searchInput);
    setPage(1);
  }

  function openCreateDialog() {
    createForm.reset({
      email: "",
      password: "",
      display_name: "",
      role: "user",
    });
    setCreateOpen(true);
  }

  const [showPassword, setShowPassword] = useState(false);

  async function handleCreate(data: CreateUserFormData) {
    try {
      await createMutation.mutateAsync({
        email: data.email,
        password: data.password,
        display_name: data.display_name || undefined,
        role: data.role,
      });
      toast.success("User created successfully");
      setCreateOpen(false);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to create user",
      );
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="User Management"
        description="View and manage all registered users."
        actions={
          canWrite ? (
            <AddCtaButton label="Create User" onClick={openCreateDialog} />
          ) : null
        }
      />

      <form onSubmit={handleSearch} className="flex items-center gap-2">
        <div className="relative max-w-sm flex-1">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Search by email..."
            value={searchInput}
            onChange={(e) => setSearchInput(e.target.value)}
            className="pl-9"
          />
        </div>
        <Button type="submit" variant="outline" size="sm">
          Search
        </Button>
        {search && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              setSearchInput("");
              setSearch("");
              setPage(1);
            }}
          >
            Clear
          </Button>
        )}
      </form>

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <Skeleton key={`user-skel-${String(i)}`} className="h-12 w-full" />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <BiometricIdentityIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground">Failed to load users</p>
            <p className="text-xs text-muted-foreground">Please try again later.</p>
          </div>
        </div>
      ) : users.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <BiometricIdentityIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground">No users found</p>
            <p className="text-xs text-muted-foreground">
              {search ? "No users match your search." : "There are no users to display."}
            </p>
          </div>
        </div>
      ) : (
        <>
          {/* Mobile card view */}
          <div className="flex flex-col gap-3 md:hidden">
            {users.map((user) => {
              const role = resolvePlatformRole(user);
              return (
                <div
                  key={user.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => void navigate({ to: "/admin/users/$userId", params: { userId: user.id } })}
                  onKeyDown={(e) => { if (e.key === "Enter") void navigate({ to: "/admin/users/$userId", params: { userId: user.id } }); }}
                  className="rounded-xl border border-border/50 bg-card p-4 transition-colors hover:bg-white/[0.03] cursor-pointer"
                >
                  <p className="text-[13px] font-semibold text-foreground truncate">{user.display_name ?? user.email}</p>
                  {user.display_name && (
                    <p className="text-[11px] text-muted-foreground truncate">{user.email}</p>
                  )}
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    <Badge variant={user.is_active ? "success" : "destructive"}>
                      {user.is_active ? "Active" : "Disabled"}
                    </Badge>
                    {role === "admin" && <Badge variant="default">Admin</Badge>}
                    {role === "operator" && <Badge variant="secondary">Operator</Badge>}
                    {user.mfa_enabled && <Badge variant="success">MFA</Badge>}
                  </div>
                  <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
                    <span>{user.email_verified ? "Verified" : "Unverified"}</span>
                    <span>{formatDate(user.last_login_at)}</span>
                  </div>
                </div>
              );
            })}
          </div>

          {/* Desktop table view */}
          <div className="hidden md:block rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Email</TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Role</TableHead>
                  <TableHead>Verified</TableHead>
                  <TableHead>MFA</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead>Last Login</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {users.map((user) => (
                  <TableRow
                    key={user.id}
                    className="cursor-pointer"
                    tabIndex={0}
                    role="link"
                    onClick={() =>
                      void navigate({
                        to: "/admin/users/$userId",
                        params: { userId: user.id },
                      })
                    }
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        void navigate({
                          to: "/admin/users/$userId",
                          params: { userId: user.id },
                        });
                      }
                    }}
                  >
                    <TableCell className="font-medium">{user.email}</TableCell>
                    <TableCell>
                      {user.display_name ?? (
                        <span className="text-muted-foreground">--</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={user.is_active ? "success" : "destructive"}
                      >
                        {user.is_active ? "Active" : "Disabled"}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      {(() => {
                        const role = resolvePlatformRole(user);
                        if (role === "admin")
                          return <Badge variant="default">Admin</Badge>;
                        if (role === "operator")
                          return <Badge variant="secondary">Operator</Badge>;
                        return <Badge variant="secondary">User</Badge>;
                      })()}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={user.email_verified ? "success" : "warning"}
                      >
                        {user.email_verified ? "Verified" : "Unverified"}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <Badge variant={user.mfa_enabled ? "success" : "secondary"}>
                        {user.mfa_enabled ? "On" : "Off"}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(user.created_at)}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(user.last_login_at)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>

          <div className="flex items-center justify-between">
            <p className="text-[11px] text-text-tertiary">
              Showing {String((page - 1) * PER_PAGE + 1)}-
              {String(Math.min(page * PER_PAGE, total))} of {String(total)}{" "}
              users
            </p>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="icon"
                disabled={page <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                aria-label="Previous page"
              >
                <ChevronLeft className="h-3 w-3" />
              </Button>
              <span className="text-[11px] text-text-tertiary">
                Page {String(page)} of {String(totalPages)}
              </span>
              <Button
                variant="outline"
                size="icon"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => p + 1)}
                aria-label="Next page"
              >
                <ChevronRight className="h-3 w-3" />
              </Button>
            </div>
          </div>
        </>
      )}
      {/* Create User Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create User</DialogTitle>
            <DialogDescription>
              Create a new user account. The account will be active and
              email-verified immediately.
            </DialogDescription>
          </DialogHeader>
          <Form {...createForm}>
            <form
              onSubmit={createForm.handleSubmit(
                (data) => void handleCreate(data),
              )}
              className="space-y-4"
            >
              <FormField
                control={createForm.control}
                name="email"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Email</FormLabel>
                    <FormControl>
                      <Input
                        type="email"
                        placeholder="user@example.com"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={createForm.control}
                name="password"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Password</FormLabel>
                    <FormControl>
                      <div className="relative">
                        <Input
                          type={showPassword ? "text" : "password"}
                          placeholder="Minimum 8 characters"
                          className="pr-9"
                          {...field}
                        />
                        <button
                          type="button"
                          tabIndex={-1}
                          onClick={() => setShowPassword((v) => !v)}
                          className="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
                          aria-label={showPassword ? "Hide password" : "Show password"}
                        >
                          {showPassword ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}
                        </button>
                      </div>
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={createForm.control}
                name="display_name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Display Name</FormLabel>
                    <FormControl>
                      <Input placeholder="Optional" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={createForm.control}
                name="role"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Role</FormLabel>
                    <Select value={field.value} onValueChange={field.onChange}>
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder="Select role" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="user">User</SelectItem>
                        <SelectItem value="operator">
                          Operator (read-only platform admin)
                        </SelectItem>
                        <SelectItem value="admin">Admin</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setCreateOpen(false)}
                >
                  Cancel
                </Button>
                <Button type="submit" variant="primary" isLoading={createMutation.isPending}>
                  Create User
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
