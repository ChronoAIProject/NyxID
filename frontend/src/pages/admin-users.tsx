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
  Users,
  Search,
  ChevronLeft,
  ChevronRight,
  Eye,
  EyeOff,
} from "lucide-react";
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
        <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
            <Users className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="space-y-1">
            <p className="text-[12px] font-medium">Failed to load users</p>
            <p className="text-xs text-muted-foreground">Please try again later.</p>
          </div>
        </div>
      ) : users.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
            <Users className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="space-y-1">
            <p className="text-[12px] font-medium">No users found</p>
            <p className="text-xs text-muted-foreground">
              {search ? "No users match your search." : "There are no users to display."}
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="rounded-xl border border-border">
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
                        return (
                          <span className="text-muted-foreground text-xs">
                            User
                          </span>
                        );
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
                      {user.mfa_enabled ? (
                        <Badge variant="success">On</Badge>
                      ) : (
                        <span className="text-muted-foreground text-xs">
                          Off
                        </span>
                      )}
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
            <p className="text-sm text-muted-foreground">
              Showing {String((page - 1) * PER_PAGE + 1)}-
              {String(Math.min(page * PER_PAGE, total))} of {String(total)}{" "}
              users
            </p>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
              >
                <ChevronLeft className="h-3 w-3" />
                Previous
              </Button>
              <span className="text-sm text-muted-foreground">
                Page {String(page)} of {String(totalPages)}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => p + 1)}
              >
                Next
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
