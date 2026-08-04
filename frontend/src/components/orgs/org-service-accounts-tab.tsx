import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import {
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  Copy,
  Plus,
  Search,
} from "lucide-react";
import { RoboticArmIcon } from "@/components/icons/empty-state";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  useCreateServiceAccount,
  useServiceAccounts,
} from "@/hooks/use-service-accounts";
import {
  createServiceAccountSchema,
  type CreateServiceAccountFormData,
} from "@/schemas/service-accounts";
import { ApiError } from "@/lib/api-client";
import { copyToClipboard, formatDate } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
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
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { OrgReadOnlyRow } from "@/components/orgs/org-readonly-row";
import type { CreateServiceAccountResponse } from "@/types/service-accounts";

const PER_PAGE = 20;

interface OrgServiceAccountsTabProps {
  readonly orgId: string;
  readonly orgName: string;
}

export function OrgServiceAccountsTab({
  orgId,
  orgName,
}: OrgServiceAccountsTabProps) {
  const navigate = useNavigate();
  const [page, setPage] = useState(1);
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [createdResult, setCreatedResult] =
    useState<CreateServiceAccountResponse | null>(null);

  const { data, isLoading, error, refetch } = useServiceAccounts(
    page,
    PER_PAGE,
    search,
    orgId,
  );
  const createMutation = useCreateServiceAccount();

  const accounts = data?.service_accounts ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / PER_PAGE));

  const createForm = useForm<CreateServiceAccountFormData>({
    resolver: zodResolver(createServiceAccountSchema),
    defaultValues: {
      name: "",
      description: "",
      allowed_scopes: "",
      role_ids: "",
      rate_limit_override: "",
    },
  });

  function handleSearch(event: React.FormEvent) {
    event.preventDefault();
    setSearch(searchInput);
    setPage(1);
  }

  function openCreateDialog() {
    createForm.reset({
      name: "",
      description: "",
      allowed_scopes: "",
      role_ids: "",
      rate_limit_override: "",
    });
    setCreatedResult(null);
    setCreateOpen(true);
  }

  function closeCreateDialog() {
    setCreateOpen(false);
    setCreatedResult(null);
  }

  async function handleCreate(formData: CreateServiceAccountFormData) {
    try {
      const roleIds = formData.role_ids
        ? formData.role_ids
            .split(",")
            .map((value) => value.trim())
            .filter(Boolean)
        : undefined;
      const rateLimit = formData.rate_limit_override
        ? Number(formData.rate_limit_override)
        : undefined;

      const result = await createMutation.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        allowed_scopes: formData.allowed_scopes,
        role_ids: roleIds,
        rate_limit_override: rateLimit,
        target_org_id: orgId,
      });
      setCreatedResult(result);
      toast.success("Service account created");
    } catch (err) {
      if (err instanceof ApiError) {
        createForm.setError("root", { message: err.message });
      } else {
        toast.error("Failed to create service account");
      }
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-2">
        <form onSubmit={handleSearch} className="flex items-center gap-2">
          <div className="relative max-w-sm flex-1">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder="Search by name..."
              value={searchInput}
              onChange={(event) => setSearchInput(event.target.value)}
              className="pl-9"
            />
          </div>
          <Button type="submit" variant="outline">
            Search
          </Button>
          {search && (
            <Button
              type="button"
              variant="ghost"
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
        <Button variant="primary" onClick={openCreateDialog}>
          <ButtonIcon><Plus className="h-4 w-4" /></ButtonIcon>
          Create Service Account
        </Button>
      </div>

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 5 }).map((_, index) => (
            <Skeleton
              key={`org-sa-skel-${String(index)}`}
              className="h-12 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="py-12">
          <ErrorBanner message={`Failed to load service accounts owned by ${orgName}.`} onRetry={refetch} />
        </div>
      ) : accounts.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <RoboticArmIcon className="h-64 w-64 text-muted-foreground/30" />
          <div className="space-y-1">
            <p className="text-[12px] font-medium text-muted-foreground/30">
              {search ? "No Results" : "No Service Accounts"}
            </p>
            <p className="text-xs text-muted-foreground/30">
              {search
                ? `No service accounts owned by ${orgName} match your search.`
                : `No service accounts owned by ${orgName}.`}
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Client ID</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Scopes</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead>Last Used</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {accounts.map((sa) => (
                  <TableRow
                    key={sa.id}
                    className="cursor-pointer"
                    tabIndex={0}
                    role="link"
                    onClick={() =>
                      void navigate({
                        to: "/orgs/$orgId/service-accounts/$saId",
                        params: { orgId, saId: sa.id },
                      })
                    }
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        void navigate({
                          to: "/orgs/$orgId/service-accounts/$saId",
                          params: { orgId, saId: sa.id },
                        });
                      }
                    }}
                  >
                    <TableCell className="font-medium">{sa.name}</TableCell>
                    <TableCell className="text-xs">
                      {sa.client_id}
                    </TableCell>
                    <TableCell>
                      <Badge variant={sa.is_active ? "success" : "destructive"}>
                        {sa.is_active ? "Active" : "Inactive"}
                      </Badge>
                    </TableCell>
                    <TableCell
                      className="max-w-[200px] truncate text-xs"
                      title={sa.allowed_scopes}
                    >
                      {sa.allowed_scopes}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(sa.created_at)}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(sa.last_authenticated_at)}
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
              service accounts
            </p>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                disabled={page <= 1}
                onClick={() => setPage((current) => Math.max(1, current - 1))}
              >
                <ChevronLeft className="h-4 w-4" />
                Previous
              </Button>
              <span className="text-[11px] text-text-tertiary">
                Page {String(page)} of {String(totalPages)}
              </span>
              <Button
                variant="outline"
                disabled={page >= totalPages}
                onClick={() => setPage((current) => current + 1)}
              >
                Next
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </>
      )}

      <Dialog open={createOpen} onOpenChange={closeCreateDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Service Account</DialogTitle>
            <DialogDescription>
              Create a machine-to-machine service account owned by {orgName}.
            </DialogDescription>
          </DialogHeader>

          {createdResult ? (
            <div className="space-y-4">
              <OrgReadOnlyRow orgName={orgName} />
              <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3">
                <div className="flex items-start gap-2">
                  <AlertTriangle className="mt-0.5 h-4 w-4 text-amber-600" />
                  <p className="text-[12px] text-amber-700 dark:text-amber-400">
                    Save these credentials now. The client secret cannot be
                    retrieved later.
                  </p>
                </div>
              </div>

              <CredentialRow
                label="Client ID"
                value={createdResult.client_id}
                copyLabel="Client ID"
              />
              <CredentialRow
                label="Client Secret"
                value={createdResult.client_secret}
                copyLabel="Client secret"
              />

              <DialogFooter>
                <Button variant="primary" onClick={closeCreateDialog}>Done</Button>
              </DialogFooter>
            </div>
          ) : (
            <Form {...createForm}>
              <form
                onSubmit={createForm.handleSubmit(
                  (data) => void handleCreate(data),
                )}
                className="space-y-4"
              >
                <OrgReadOnlyRow orgName={orgName} />
                {createForm.formState.errors.root && (
                  <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
                    {createForm.formState.errors.root.message}
                  </div>
                )}
                <FormField
                  control={createForm.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Name</FormLabel>
                      <FormControl>
                        <Input placeholder="e.g. CI/CD Pipeline" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="description"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Description</FormLabel>
                      <FormControl>
                        <Input placeholder="Optional description" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="allowed_scopes"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Allowed Scopes</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="e.g. openid proxy:* llm:proxy"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="role_ids"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        Role IDs (comma-separated, optional)
                      </FormLabel>
                      <FormControl>
                        <Input placeholder="Optional" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={createForm.control}
                  name="rate_limit_override"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Rate Limit Override (optional)</FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          placeholder="Requests per second"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <DialogFooter>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={closeCreateDialog}
                  >
                    Cancel
                  </Button>
                  <Button variant="primary" type="submit" isLoading={createMutation.isPending} disabled={!createForm.formState.isValid || createMutation.isPending}>
                    Create
                  </Button>
                </DialogFooter>
              </form>
            </Form>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

function CredentialRow({
  label,
  value,
  copyLabel,
}: {
  readonly label: string;
  readonly value: string;
  readonly copyLabel: string;
}) {
  return (
    <div>
      <p className="mb-1 text-xs font-medium text-muted-foreground">{label}</p>
      <div className="flex items-center gap-2">
        <code className="flex-1 break-all rounded bg-muted px-2 py-1 font-mono text-[12px]">
          {value}
        </code>
        <Button
          variant="outline"
          size="icon"
          className="h-8 w-8"
          onClick={() =>
            void copyToClipboard(value).then(
              () => toast.success(`${copyLabel} copied`),
              () => toast.error("Failed to copy"),
            )
          }
        >
          <Copy className="h-3 w-3" />
        </Button>
      </div>
    </div>
  );
}
