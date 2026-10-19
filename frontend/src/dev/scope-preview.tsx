import { useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ServiceAccountScopePicker } from "@/components/service-accounts/service-account-scope-picker";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Form, FormControl, FormField, FormItem, FormLabel, useAppForm } from "@/components/ui/form";
import { useAuthStore } from "@/stores/auth-store";
import "@/app.css";

const ownerId = "67b1ec25-a2d4-4615-8979-d7eeac100001";
const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });

export function ScopePreview() {
  const form = useAppForm({ defaultValues: { name: "Build pipeline", allowed_scopes: "roles" } });
  const [scenario, setScenario] = useState("available");
  const [changingScenario, setChangingScenario] = useState(false);
  const [previewError, setPreviewError] = useState("");
  const [saved, setSaved] = useState<{ name: string; allowed_scopes: string } | null>(null);

  async function changeScenario(value: string) {
    setChangingScenario(true);
    setPreviewError("");
    try {
      const response = await fetch(`/__scope_preview__/scenario?value=${value}`, { method: "POST" });
      if (!response.ok) throw new Error("Could not change preview scenario.");
      setScenario(value);
      await queryClient.resetQueries({ queryKey: ["options"] });
    } catch {
      setPreviewError("Could not change preview scenario. Try again.");
    } finally {
      setChangingScenario(false);
    }
  }

  return <main className="theme-dark min-h-screen bg-background px-4 py-8 text-foreground sm:px-8 sm:py-12">
    <div className="mx-auto max-w-5xl space-y-8">
      <header className="space-y-3">
        <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">NyxID · Local preview · Mock data</p>
        <h1 className="text-3xl font-semibold tracking-tight">Service account scopes</h1>
        <p className="max-w-2xl text-sm leading-6 text-muted-foreground">Build a scope from suggestions or type your own in the same field. Saving only updates this preview.</p>
      </header>
      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_280px]">
        <section className="min-w-0 rounded-xl border bg-card p-5 sm:p-7" aria-label="Service account form">
          <Form {...form}>
            <form className="space-y-6" onSubmit={form.handleSubmit(setSaved)}>
              <FormField control={form.control} name="name" render={({ field }) => <FormItem>
                <FormLabel>Account name</FormLabel><FormControl><Input {...field} /></FormControl>
              </FormItem>} />
              <FormField control={form.control} name="allowed_scopes" render={({ field }) => <FormItem>
                <FormLabel>Allowed scopes</FormLabel>
                <FormControl><ServiceAccountScopePicker ownerId={ownerId} {...field} /></FormControl>
              </FormItem>} />
              <div className="flex flex-wrap items-center gap-3 border-t pt-5">
                <Button type="submit" disabled={!form.formState.isDirty || !form.watch("allowed_scopes").trim()}>Save preview</Button>
                <Button type="button" variant="outline" onClick={() => { form.reset(); setSaved(null); }}>Reset form</Button>
                <Button type="button" variant="ghost" onClick={() => {
                  form.reset({ name: "Reporting agent", allowed_scopes: "roles reports:read reports:finance:read metrics:read custom:billing:invoices:approve" });
                  setSaved(null);
                }}>Load example account</Button>
              </div>
              {saved && <div role="status" className="space-y-2 rounded-lg border border-primary/30 bg-primary/5 p-4 text-sm">
                <p className="font-medium">Preview saved</p><p className="text-muted-foreground">{saved.name}</p>
                <code className="block whitespace-pre-wrap break-all" data-testid="saved-scopes">{saved.allowed_scopes}</code>
              </div>}
            </form>
          </Form>
        </section>
        <aside className="space-y-5 rounded-xl border bg-card p-5">
          <div className="space-y-2">
            <label htmlFor="suggestion-scenario" className="text-sm font-medium">Suggestion source</label>
            <select id="suggestion-scenario" className="h-10 w-full rounded-md border bg-background px-3 text-sm" value={scenario} disabled={changingScenario} onChange={(event) => void changeScenario(event.target.value)}>
              <option value="available">Available suggestions</option><option value="empty">Empty suggestion list</option><option value="unavailable">Suggestions unavailable</option>
            </select>
            {previewError && <p role="alert" className="text-sm text-destructive">{previewError}</p>}
          </div>
          <div className="space-y-3 text-sm leading-6 text-muted-foreground">
            <p>Type <code className="text-foreground">reports</code>, choose <code className="text-foreground">reports:</code>, then choose a suggested next segment. Try <code className="text-foreground">reports:finance:read</code>.</p>
            <p>Or type a complete scope, such as <code className="text-foreground">inventory:read</code>, and press Enter. You can also paste several scopes separated by spaces.</p>
            <p>Click a scope pill to edit its value. Press Enter to apply the change or Escape to cancel. All selected scopes stay visible.</p>
            <p>Switch to an empty or unavailable source and continue entering custom scopes.</p>
          </div>
          <p className="border-t pt-4 text-xs leading-5 text-muted-foreground">Mock suggestions are examples. Custom scopes are saved as entered; supported permissions depend on the service handling them.</p>
        </aside>
      </div>
    </div>
  </main>;
}

if (import.meta.env.DEV) {
  useAuthStore.setState({ user: { id: ownerId, email: "preview@example.test", display_name: "Local preview", avatar_url: null, email_verified: true, mfa_enabled: false, is_admin: true, is_active: true, created_at: "2026-09-17T00:00:00Z" }, isAuthenticated: true, isLoading: false });
  const container = document.getElementById("root");
  if (container) {
    const root = import.meta.hot?.data.root ?? createRoot(container);
    if (import.meta.hot) import.meta.hot.data.root = root;
    root.render(<QueryClientProvider client={queryClient}><ScopePreview /></QueryClientProvider>);
  }
}
