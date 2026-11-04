import { createRoot } from "react-dom/client";
import { useEffect, useState } from "react";
import { useApplyTheme } from "../../src/hooks/use-theme";
import { useThemeStore } from "../../src/stores/theme-store";
import { TEXT_STYLES } from "../../src/lib/typography";
import { useAppForm, Form, FormField, FormItem, FormLabel, FormControl } from "../../src/components/ui/form";
import { GroupedField, GroupedLabel } from "../../src/components/ui/grouped-fields";
import { GroupedList, GroupedInput } from "../../src/components/ui/grouped-list";
import { Input } from "../../src/components/ui/input";
import { Button } from "../../src/components/ui/button";
import { Card, CardTitle, CardDescription } from "../../src/components/ui/card";
import { PageHeader } from "../../src/components/shared/page-header";
import { Dialog, DialogContent, DialogTitle, DialogDescription } from "../../src/components/ui/dialog";
import { Sheet, SheetContent, SheetTitle, SheetDescription } from "../../src/components/ui/sheet";
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from "../../src/components/ui/dropdown-menu";
import { Select, SelectTrigger, SelectContent, SelectValue, SelectItem } from "../../src/components/ui/select";
import { Tooltip, TooltipProvider, TooltipTrigger, TooltipContent } from "../../src/components/ui/tooltip";
import { ToastViewport } from "../../src/components/ui/toast";
import { DisplaySettings } from "../../src/components/settings/display-settings";
import { toast } from "sonner";
import "../../src/app.css";

declare global {
  interface Window {
    displayFixture: {
      set: (scale: number, mode: "light" | "dark") => void;
      portal: (kind: string) => void;
    };
  }
}

export function Fixture() {
  const assistant = location.pathname.startsWith("/assistant");
  useApplyTheme(assistant ? "assistant" : undefined);
  const [kind, setKind] = useState("");
  const form = useAppForm({ defaultValues: { email: "reader@example.test" } });
  useEffect(() => { window.displayFixture = {
    set: (textScale, mode) => useThemeStore.setState({ textScale, mode }),
    portal: (next) => {
      setKind(next);
      toast.dismiss();
      if (next === "toast") toast("Toast title", { description: "Toast description", duration: Infinity, action: { label: "Action", onClick() {} }, cancel: { label: "Cancel", onClick() {} } });
    },
  }; }, []);
  return <main className="space-y-4 p-4">
    <PageHeader title="Display test" description="Page description" />
    <Card data-role="card" className="p-4"><CardTitle>Card title</CardTitle><CardDescription>Card subtitle</CardDescription></Card>
    <Form {...form}><form aria-label="Profile form" className="space-y-4">
      <FormField control={form.control} name="email" render={({ field }) => assistant ? <GroupedList title="Profile"><GroupedField><GroupedLabel>Email</GroupedLabel><FormControl><GroupedInput {...field} /></FormControl></GroupedField></GroupedList> : <FormItem><FormLabel>Email</FormLabel><FormControl><Input {...field} /></FormControl></FormItem>} />
      <Button type="submit">Save changes</Button>
    </form></Form>
    <p data-role="body" className={TEXT_STYLES.body}>Body text</p><p data-role="meta" className={TEXT_STYLES.meta}>Small meta</p>
    <DisplaySettings />
    <ToastViewport position="top-right" />
    <Dialog open={kind === "dialog"} onOpenChange={() => setKind("")}><DialogContent data-fixture-portal="dialog"><DialogTitle>Dialog title</DialogTitle><DialogDescription>Dialog description</DialogDescription><Button>Save changes</Button></DialogContent></Dialog>
    <Sheet open={kind === "sheet"} onOpenChange={() => setKind("")}><SheetContent data-fixture-portal="sheet"><SheetTitle>Sheet title</SheetTitle><SheetDescription>Sheet description</SheetDescription></SheetContent></Sheet>
    <DropdownMenu open={kind === "menu"} onOpenChange={() => {}}><DropdownMenuTrigger asChild><Button>Menu</Button></DropdownMenuTrigger><DropdownMenuContent data-fixture-portal="menu"><DropdownMenuItem>Menu item</DropdownMenuItem></DropdownMenuContent></DropdownMenu>
    <Select open={kind === "select"} onOpenChange={() => {}} value="one"><SelectTrigger><SelectValue /></SelectTrigger><SelectContent data-fixture-portal="select"><SelectItem value="one">Select option</SelectItem></SelectContent></Select>
    <TooltipProvider><Tooltip open={kind === "tooltip"}><TooltipTrigger asChild><Button>Help</Button></TooltipTrigger><TooltipContent data-fixture-portal="tooltip">Tooltip text</TooltipContent></Tooltip></TooltipProvider>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
