import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  createRegistrationTokenSchema,
  type CreateRegistrationTokenFormData,
} from "@/schemas/nodes";
import { useNodes, useCreateRegistrationToken, useDeleteNode } from "@/hooks/use-nodes";
import { ApiError } from "@/lib/api-client";
import { formatRelativeTime } from "@/lib/utils";
import { PageHeader } from "@/components/shared/page-header";
import { CopyableField } from "@/components/shared/copyable-field";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
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
  HardDrive,
  Plus,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import { NodeStatusBadge } from "@/components/shared/node-status-badge";

function RegisterNodeDialog() {
  const [open, setOpen] = useState(false);
  const [createdToken, setCreatedToken] = useState<{
    readonly token: string;
    readonly name: string;
    readonly expires_at: string;
  } | null>(null);
  const createMutation = useCreateRegistrationToken();

  const form = useForm<CreateRegistrationTokenFormData>({
    resolver: zodResolver(createRegistrationTokenSchema),
    defaultValues: { name: "" },
  });

  async function onSubmit(data: CreateRegistrationTokenFormData) {
    try {
      const result = await createMutation.mutateAsync(data);
      setCreatedToken({
        token: result.token,
        name: result.name,
        expires_at: result.expires_at,
      });
      toast.success("Registration token created");
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to create registration token");
      }
    }
  }

  function handleClose() {
    setOpen(false);
    setCreatedToken(null);
    form.reset();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => (o ? setOpen(true) : handleClose())}
    >
      <DialogTrigger asChild>
        <Button>
          <Plus className="mr-2 h-4 w-4" />
          Register Node
        </Button>
      </DialogTrigger>
      <DialogContent>
        {createdToken ? (
          <>
            <DialogHeader>
              <DialogTitle>Registration Token Created</DialogTitle>
              <DialogDescription>
                Copy the token and use it to register your node. This token will
                not be shown again and expires at{" "}
                {new Date(createdToken.expires_at).toLocaleString()}.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-3">
              <CopyableField label="Registration Token" value={createdToken.token} />
              <div className="rounded-md bg-muted p-3">
                <p className="mb-1 text-xs font-medium text-text-tertiary">
                  Run on your node
                </p>
                <code className="text-xs text-foreground break-all">
                  nyxid-node register --token {createdToken.token}
                </code>
              </div>
            </div>
            <DialogFooter>
              <Button onClick={handleClose}>Done</Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>Register a New Node</DialogTitle>
              <DialogDescription>
                Create a registration token to connect a new credential node.
              </DialogDescription>
            </DialogHeader>
            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onSubmit)}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                    {form.formState.errors.root.message}
                  </div>
                )}
                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Node Name</FormLabel>
                      <FormControl>
                        <Input placeholder="my-home-server" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={handleClose}>
                    Cancel
                  </Button>
                  <Button type="submit" isLoading={createMutation.isPending}>
                    Create Token
                  </Button>
                </DialogFooter>
              </form>
            </Form>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function NodesPage() {
  const { data: nodes, isLoading, error } = useNodes();
  const deleteMutation = useDeleteNode();
  const [deleteTarget, setDeleteTarget] = useState<{
    readonly id: string;
    readonly name: string;
  } | null>(null);

  async function handleDelete() {
    if (!deleteTarget) return;
    try {
      await deleteMutation.mutateAsync(deleteTarget.id);
      toast.success(`Node "${deleteTarget.name}" deleted`);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to delete node",
      );
    } finally {
      setDeleteTarget(null);
    }
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title="Credential Nodes"
        description="Manage your credential nodes for self-hosted proxy routing."
        actions={<RegisterNodeDialog />}
      />

      {isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton
              key={`node-skel-${String(i)}`}
              className="h-16 w-full"
            />
          ))}
        </div>
      ) : error ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <HardDrive className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Failed to load nodes. Please try again.
          </p>
        </div>
      ) : !nodes || nodes.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <HardDrive className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No credential nodes registered. Create a registration token to get
            started.
          </p>
        </div>
      ) : (
        <div className="rounded-xl border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Bindings</TableHead>
                <TableHead>Last Heartbeat</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="w-[80px]" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {nodes.map((node) => (
                <TableRow key={node.id}>
                  <TableCell>
                    <Link
                      to="/nodes/$nodeId"
                      params={{ nodeId: node.id }}
                      className="font-medium text-foreground hover:underline"
                    >
                      {node.name}
                    </Link>
                    {node.metadata?.agent_version && (
                      <span className="ml-2 text-xs text-muted-foreground">
                        v{node.metadata.agent_version}
                      </span>
                    )}
                  </TableCell>
                  <TableCell>
                    <NodeStatusBadge status={node.status} isConnected={node.is_connected} />
                  </TableCell>
                  <TableCell>
                    <span className="text-sm text-muted-foreground">
                      {String(node.binding_count)} service
                      {node.binding_count !== 1 ? "s" : ""}
                    </span>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatRelativeTime(node.last_heartbeat_at) ?? "Never"}
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatRelativeTime(node.created_at)}
                  </TableCell>
                  <TableCell>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8 text-muted-foreground hover:text-destructive"
                      onClick={() =>
                        setDeleteTarget({ id: node.id, name: node.name })
                      }
                    >
                      <Trash2 className="h-4 w-4" />
                      <span className="sr-only">Delete {node.name}</span>
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {/* Delete Confirmation Dialog */}
      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Node</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete &quot;{deleteTarget?.name ?? ""}
              &quot;? This will disconnect the node and remove all its service
              bindings. This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
