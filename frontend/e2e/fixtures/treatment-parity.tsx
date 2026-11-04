import { useEffect, useState } from "react";
import { Search, Check } from "lucide-react";
import { toast } from "sonner";
import { useThemeStore } from "../../src/stores/theme-store";
import { Button, type ButtonProps } from "../../src/components/ui/button";
import { Input } from "../../src/components/ui/input";
import { Textarea } from "../../src/components/ui/textarea";
import { Label } from "../../src/components/ui/label";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from "../../src/components/ui/card";
import { Badge } from "../../src/components/ui/badge";
import { Switch } from "../../src/components/ui/switch";
import { Checkbox } from "../../src/components/ui/checkbox";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "../../src/components/ui/tabs";
import { SegmentedControl, SegmentedControlList, SegmentedControlTrigger } from "../../src/components/ui/segmented-control";
import { GroupedList, GroupedRow, GroupedDisclosure, GroupedInput, GroupedTextarea, GroupedSelectTrigger } from "../../src/components/ui/grouped-list";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "../../src/components/ui/select";
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from "../../src/components/ui/dropdown-menu";
import { Popover, PopoverTrigger, PopoverContent } from "../../src/components/ui/popover";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogBody, DialogFooter } from "../../src/components/ui/dialog";
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetDescription, SheetFooter } from "../../src/components/ui/sheet";
import { Tooltip, TooltipProvider, TooltipTrigger, TooltipContent } from "../../src/components/ui/tooltip";
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from "../../src/components/ui/table";
import { PageHeader } from "../../src/components/shared/page-header";
import { WorkspacePageHeader } from "../../src/components/assistant/workspace-page-header";

declare global {
  interface Window {
    treatmentFixture: {
      set: (scale: number, density: number, mode: "light" | "dark") => void;
      portal: (kind: string) => void;
    };
  }
}
const variants: NonNullable<ButtonProps["variant"]>[] = ["default", "primary", "secondary", "ghost", "outline", "destructive", "link"];
const sizes: NonNullable<ButtonProps["size"]>[] = ["default", "sm", "lg", "icon"];

export function TreatmentFixture() {
  const [kind, setKind] = useState("");
  const assistant = location.pathname.startsWith("/assistant");
  const Header = assistant ? WorkspacePageHeader : PageHeader;
  useEffect(() => {
    window.treatmentFixture = {
      set: (textScale, density, mode) => useThemeStore.setState({ textScale, density, mode }),
      portal: next => {
        toast.dismiss();
        setKind(next);
        if (next === "toast" || next.startsWith("toast:")) {
          const variants: Record<string, typeof toast.success> = { success: toast.success, error: toast.error, info: toast.info, warning: toast.warning };
          const show = variants[next.slice(6)] ?? toast;
          show("Review completed", { description: "Review these changes.", duration: Infinity, action: { label: "Review changes", onClick() {} }, cancel: { label: "Dismiss", onClick() {} } });
        }
      },
    };
  }, []);
  return <section data-treatment-fixture="" className="shell-page-stack mx-auto w-full max-w-[20rem]">
    <Header title="Settings" description="Shared page description" actions={<Button>Update</Button>} />
    <div className="flex flex-wrap gap-2">
      {variants.flatMap(variant => sizes.map(size => <Button key={`${variant}-${size}`} variant={variant} size={size} data-parity={`button-${variant}-${size}`} aria-label={size === "icon" ? `${variant} icon` : undefined}><Check />{size !== "icon" && "Save"}</Button>))}
      <Button data-parity="button-text">Save</Button>
      <button data-parity="custom-icon" data-icon-button="" aria-label="Search"><Search /></button>
      <Button data-parity="hidden-icon" size="icon" className="hidden" aria-label="Hidden action"><Search /></Button>
    </div>
    <div data-parity="account-block" className="shell-user-block"><div data-parity="account-avatar" className="shell-user-avatar"><Search /></div><div data-parity="account-copy" className="shell-user-copy"><p data-parity="account-name" className="shell-user-name">Reader</p><p data-parity="account-email" className="shell-user-email">reader@example.test</p></div></div>
    <Label data-parity="label" htmlFor="parity-name">Name</Label>
    <Input data-parity="input" id="parity-name" defaultValue="Reader" />
    <Textarea data-parity="textarea" defaultValue="Notes" />
    <div data-parity="search" className="flex items-center gap-2 rounded-full bg-muted px-3 py-1 text-14"><Search className="size-[1.125em]" /><Input aria-label="Search entries" className="border-0 px-0" /></div>
    <Select value="one"><SelectTrigger data-parity="select-trigger"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="one">Option</SelectItem></SelectContent></Select>
    <Badge data-parity="badge">Active</Badge>
    <Switch data-parity="switch" defaultChecked aria-label="Enabled" />
    <Checkbox data-parity="checkbox" defaultChecked aria-label="Selected" />
    <Tabs defaultValue="one"><TabsList data-parity="tabs"><TabsTrigger data-parity="tab" value="one">First</TabsTrigger><TabsTrigger value="two">Second</TabsTrigger></TabsList><TabsContent value="one">Tab content</TabsContent></Tabs>
    <SegmentedControl defaultValue="one"><SegmentedControlList data-parity="segments"><SegmentedControlTrigger data-parity="segment" value="one">First</SegmentedControlTrigger><SegmentedControlTrigger value="two">Second</SegmentedControlTrigger></SegmentedControlList></SegmentedControl>
    <Card data-parity="card"><CardHeader data-parity="card-header"><CardTitle>Card title</CardTitle><CardDescription>Card description</CardDescription></CardHeader><CardContent data-parity="card-body">Body</CardContent><CardFooter data-parity="card-footer"><Button>Save</Button></CardFooter></Card>
    <GroupedList title="Profile" footer="Shared group guidance.">
      <GroupedRow data-parity="grouped-row" title="Notifications" description="Row subtitle" accessory={<Switch aria-label="Notifications" />} />
      <GroupedRow><GroupedInput data-parity="grouped-input" aria-label="Display name" defaultValue="Reader" /></GroupedRow>
      <GroupedRow><GroupedTextarea data-parity="grouped-textarea" aria-label="Notes" defaultValue="Notes" /></GroupedRow>
      <GroupedRow><Select value="one"><GroupedSelectTrigger data-parity="grouped-select"><SelectValue /></GroupedSelectTrigger><SelectContent><SelectItem value="one">Option</SelectItem></SelectContent></Select></GroupedRow>
      <GroupedDisclosure data-parity="grouped-disclosure">Details</GroupedDisclosure>
    </GroupedList>
    <Table data-parity="table"><TableHeader><TableRow><TableHead data-parity="table-head">Service</TableHead></TableRow></TableHeader><TableBody><TableRow><TableCell data-parity="table-cell">Calendar</TableCell></TableRow></TableBody></Table>
    <Dialog open={kind === "dialog"} onOpenChange={() => setKind("")}><DialogContent data-parity="dialog" scrollMode="body"><DialogHeader data-parity="dialog-header"><DialogTitle>Review changes</DialogTitle><DialogDescription>Check the changes.</DialogDescription></DialogHeader><DialogBody data-parity="dialog-body"><Input aria-label="Dialog name" defaultValue="Reader" /></DialogBody><DialogFooter data-parity="dialog-footer"><Button variant="ghost">Cancel</Button><Button variant="primary">Save changes</Button></DialogFooter></DialogContent></Dialog>
    <Sheet open={kind === "sheet"} onOpenChange={() => setKind("")}><SheetContent data-parity="sheet"><SheetHeader data-parity="sheet-header"><SheetTitle>Settings</SheetTitle><SheetDescription>Shared settings</SheetDescription></SheetHeader><div data-parity="sheet-body" className="p-4"><Input aria-label="Sheet name" defaultValue="Reader" /></div><SheetFooter data-parity="sheet-footer"><Button>Save</Button></SheetFooter></SheetContent></Sheet>
    <DropdownMenu open={kind === "menu"} onOpenChange={() => {}}><DropdownMenuTrigger asChild><Button>Menu</Button></DropdownMenuTrigger><DropdownMenuContent data-parity="menu"><DropdownMenuItem data-parity="menu-item"><Check />Save changes</DropdownMenuItem></DropdownMenuContent></DropdownMenu>
    <Popover open={kind === "popover"} onOpenChange={() => {}}><PopoverTrigger asChild><Button>Options</Button></PopoverTrigger><PopoverContent data-parity="popover"><Button>Save</Button></PopoverContent></Popover>
    <Select open={kind === "select"} onOpenChange={() => {}} value="one"><SelectTrigger><SelectValue /></SelectTrigger><SelectContent data-parity="select-content"><SelectItem data-parity="select-item" value="one">Option</SelectItem></SelectContent></Select>
    <TooltipProvider><Tooltip open={kind === "tooltip"}><TooltipTrigger asChild><Button>Help</Button></TooltipTrigger><TooltipContent data-parity="tooltip">Tooltip text</TooltipContent></Tooltip></TooltipProvider>
  </section>;
}
