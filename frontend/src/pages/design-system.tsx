import { useState } from "react";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Label } from "@/components/ui/label";
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
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { toast } from "sonner";
import {
  Plus,
  Settings,
  ChevronDown,
  Copy,
  Trash2,
  Edit,
  Check,
  X,
  AlertCircle,
  Info,
  Cable,
  KeyRound,
  Server,
  ArrowRight,
  Search,
  ShieldCheck,
  CheckCircle2,
  ShieldAlert,
  Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";

function Section({
  id,
  title,
  children,
}: {
  readonly id: string;
  readonly title: string;
  readonly children: React.ReactNode;
}) {
  return (
    <section id={id} className="scroll-mt-8">
      <h2 className="text-[24px] font-bold tracking-tight mb-6" style={{ letterSpacing: "-0.02em" }}>
        {title}
      </h2>
      {children}
    </section>
  );
}

function Swatch({ name, cls, textCls }: { readonly name: string; readonly cls: string; readonly textCls?: string }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className={cn("h-12 w-full rounded-lg border border-white/[0.06]", cls)} />
      <span className={cn("text-[11px] font-mono", textCls ?? "text-text-tertiary")}>{name}</span>
    </div>
  );
}

function TokenRow({ token, value, preview }: { readonly token: string; readonly value: string; readonly preview?: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between py-2 border-b border-border/20 last:border-0">
      <code className="text-[12px] font-mono text-nyx-secondary-400">{token}</code>
      <div className="flex items-center gap-3">
        {preview}
        <span className="text-[12px] text-muted-foreground">{value}</span>
      </div>
    </div>
  );
}

function ComponentShowcase({ title, children }: { readonly title: string; readonly children: React.ReactNode }) {
  return (
    <div className="space-y-3">
      <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">{title}</p>
      <div className="rounded-xl border border-border/50 bg-card p-6">
        {children}
      </div>
    </div>
  );
}

const DS_NAV = [
  { id: "colors", label: "Colors" },
  { id: "typography", label: "Typography" },
  { id: "spacing", label: "Spacing & Radius" },
  { id: "buttons", label: "Buttons" },
  { id: "inputs", label: "Inputs" },
  { id: "cards", label: "Cards" },
  { id: "badges", label: "Badges" },
  { id: "tabs", label: "Tabs" },
  { id: "tables", label: "Tables" },
  { id: "dropdowns", label: "Dropdowns" },
  { id: "dialogs", label: "Dialogs" },
  { id: "tooltips", label: "Tooltips" },
  { id: "switches", label: "Switches" },
  { id: "skeletons", label: "Skeletons" },
  { id: "icons", label: "Icon Styling" },
  { id: "toasts", label: "Toasts" },
  { id: "patterns", label: "Patterns" },
  { id: "ux-rules", label: "UX Rules" },
];

export function DesignSystemPage() {
  const [switchVal, setSwitchVal] = useState(true);

  return (
    <div className="min-h-dvh bg-background text-foreground">
    <div className="mx-auto max-w-[1200px] flex gap-10 px-8 py-10">
      {/* Sticky nav */}
      <nav className="hidden xl:flex shrink-0 w-[180px] flex-col gap-1 sticky top-10 self-start">
        <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary mb-2">Components</p>
        {DS_NAV.map((item) => (
          <a
            key={item.id}
            href={`#${item.id}`}
            className="text-[13px] text-muted-foreground hover:text-foreground transition-colors duration-300 py-1"
          >
            {item.label}
          </a>
        ))}
      </nav>

      {/* Main content */}
      <div className="flex-1 min-w-0 space-y-16">
        {/* Header */}
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-nyx-secondary-400 mb-2">Design System</p>
          <h1 className="text-[28px] font-bold leading-[1.1]" style={{ letterSpacing: "-0.03em" }}>
            NyxID Components
          </h1>
          <p className="text-[12px] text-muted-foreground mt-2">
            The canonical reference for every UI primitive. All components in the product must follow these patterns.
          </p>
        </div>

        {/* ─── COLORS ─── */}
        <Section id="colors" title="Colors">
          <div className="space-y-6">
            <ComponentShowcase title="Brand Primary">
              <div className="grid grid-cols-5 sm:grid-cols-10 gap-3">
                <Swatch name="50" cls="bg-nyx-50" />
                <Swatch name="100" cls="bg-nyx-100" />
                <Swatch name="200" cls="bg-nyx-200" />
                <Swatch name="300" cls="bg-nyx-300" />
                <Swatch name="400" cls="bg-nyx-400" />
                <Swatch name="500" cls="bg-nyx-500" textCls="text-nyx-secondary-400" />
                <Swatch name="600" cls="bg-nyx-600" />
                <Swatch name="700" cls="bg-nyx-700" />
                <Swatch name="800" cls="bg-nyx-800" />
                <Swatch name="900" cls="bg-nyx-900" />
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Brand Secondary">
              <div className="grid grid-cols-5 sm:grid-cols-10 gap-3">
                <Swatch name="50" cls="bg-nyx-secondary-50" />
                <Swatch name="100" cls="bg-nyx-secondary-100" />
                <Swatch name="200" cls="bg-nyx-secondary-200" />
                <Swatch name="300" cls="bg-nyx-secondary-300" />
                <Swatch name="400" cls="bg-nyx-secondary-400" textCls="text-nyx-secondary-400" />
                <Swatch name="500" cls="bg-nyx-secondary-500" />
                <Swatch name="600" cls="bg-nyx-secondary-600" />
                <Swatch name="700" cls="bg-nyx-secondary-700" />
                <Swatch name="800" cls="bg-nyx-secondary-800" />
                <Swatch name="900" cls="bg-nyx-secondary-900" />
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Semantic">
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                <Swatch name="background" cls="bg-background" />
                <Swatch name="card" cls="bg-card" />
                <Swatch name="surface" cls="bg-surface" />
                <Swatch name="muted" cls="bg-muted" />
                <Swatch name="success" cls="bg-success" />
                <Swatch name="warning" cls="bg-warning" />
                <Swatch name="destructive" cls="bg-destructive" />
                <Swatch name="border" cls="bg-border" />
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Border Opacity Scale">
              <div className="flex flex-col gap-2">
                <TokenRow token="border-white/[0.08]" value="Passive — default borders for chrome elements" preview={<div className="h-8 w-24 rounded-lg border border-white/[0.08]" />} />
                <TokenRow token="border-white/[0.15]" value="Hover — elevated state on interactive borders" preview={<div className="h-8 w-24 rounded-lg border border-white/[0.15]" />} />
                <TokenRow token="border-border/50" value="Cards — subtle card boundaries" preview={<div className="h-8 w-24 rounded-lg border border-border/50 bg-card" />} />
                <TokenRow token="border-border/30" value="Layout — sidebar & header dividers" preview={<div className="h-8 w-24 rounded-lg border border-border/30" />} />
                <TokenRow token="border-nyx-500/20" value="Accent — primary brand cards" preview={<div className="h-8 w-24 rounded-lg border border-nyx-500/20 bg-nyx-500/[0.04]" />} />
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── TYPOGRAPHY ─── */}
        <Section id="typography" title="Typography">
          <ComponentShowcase title="Scale">
            <div className="space-y-6">
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Page Title — 48px / bold / -0.035em</p>
                <h1 className="text-[28px] font-bold leading-[1.1]" style={{ letterSpacing: "-0.03em" }}>Page Title</h1>
              </div>
              <Separator />
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Section Title — 24px / bold / -0.02em</p>
                <h2 className="text-[24px] font-bold" style={{ letterSpacing: "-0.02em" }}>Section Title</h2>
              </div>
              <Separator />
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Card Title — 16px (base) / semibold</p>
                <h3 className="text-base font-semibold">Card Title</h3>
              </div>
              <Separator />
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Body — 14px (text-sm) / normal</p>
                <p className="text-sm">Body text for descriptions and content paragraphs.</p>
              </div>
              <Separator />
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Secondary — 13px / normal / text-muted-foreground</p>
                <p className="text-[13px] text-muted-foreground">Secondary text for compact descriptions and dashboard UI.</p>
              </div>
              <Separator />
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Small — 12px / normal</p>
                <p className="text-[12px]">Small text for table cells, info rows, and metadata.</p>
              </div>
              <Separator />
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Overline — 11px / semibold / uppercase / tracking-[1.5px] / text-text-tertiary</p>
                <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">Section Overline</p>
              </div>
              <Separator />
              <div>
                <p className="text-[11px] text-text-tertiary mb-1">Caption — 10px / medium — badges only</p>
                <p className="text-[10px] font-medium">Badge caption text</p>
              </div>
            </div>
          </ComponentShowcase>

          <div className="mt-6">
            <ComponentShowcase title="Text Colors">
              <div className="space-y-2">
                <p className="text-foreground text-[13px]"><code className="text-nyx-secondary-400 text-[11px] font-mono mr-3">text-foreground</code> Primary text — headings, titles, important content</p>
                <p className="text-muted-foreground text-[13px]"><code className="text-nyx-secondary-400 text-[11px] font-mono mr-3">text-muted-foreground</code> Secondary text — descriptions, body copy</p>
                <p className="text-text-tertiary text-[13px]"><code className="text-nyx-secondary-400 text-[11px] font-mono mr-3">text-text-tertiary</code> Tertiary text — placeholders, overlines, captions</p>
                <p className="text-nyx-secondary-400 text-[13px]"><code className="text-nyx-secondary-400 text-[11px] font-mono mr-3">text-nyx-secondary-400</code> Accent text — links, active states</p>
                <p className="text-success text-[13px]"><code className="text-nyx-secondary-400 text-[11px] font-mono mr-3">text-success</code> Success state</p>
                <p className="text-warning text-[13px]"><code className="text-nyx-secondary-400 text-[11px] font-mono mr-3">text-warning</code> Warning state</p>
                <p className="text-destructive text-[13px]"><code className="text-nyx-secondary-400 text-[11px] font-mono mr-3">text-destructive</code> Destructive / error state</p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── SPACING & RADIUS ─── */}
        <Section id="spacing" title="Spacing & Radius">
          <div className="space-y-6">
            <ComponentShowcase title="Border Radius">
              <div className="flex flex-wrap gap-6 items-end">
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-xl border border-border bg-muted" />
                  <code className="text-[11px] font-mono text-text-tertiary">rounded-xl</code>
                  <span className="text-[10px] text-text-tertiary">Cards, dialogs, selects</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-lg border border-border bg-muted" />
                  <code className="text-[11px] font-mono text-text-tertiary">rounded-lg</code>
                  <span className="text-[10px] text-text-tertiary">Buttons, inputs, items</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-[8px] border border-border bg-muted" />
                  <code className="text-[11px] font-mono text-text-tertiary">rounded-[8px]</code>
                  <span className="text-[10px] text-text-tertiary">Icon containers</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-[6px] border border-border bg-muted" />
                  <code className="text-[11px] font-mono text-text-tertiary">rounded-[6px]</code>
                  <span className="text-[10px] text-text-tertiary">Small elements, kbds</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-[10px] border border-border bg-muted" />
                  <code className="text-[11px] font-mono text-text-tertiary">rounded-[10px]</code>
                  <span className="text-[10px] text-text-tertiary">Badges (pill)</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-full border border-border bg-muted" />
                  <code className="text-[11px] font-mono text-text-tertiary">rounded-full</code>
                  <span className="text-[10px] text-text-tertiary">Status dots, chips</span>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Spacing Tokens">
              <div className="flex flex-col gap-2">
                <TokenRow token="gap-8" value="Between page sections" />
                <TokenRow token="gap-6" value="Between card groups / major elements" />
                <TokenRow token="gap-4" value="Within cards (internal content)" />
                <TokenRow token="gap-3" value="Grid items, compact lists" />
                <TokenRow token="gap-2" value="Between list items, form fields" />
                <TokenRow token="gap-1.5" value="Between chips, tight elements" />
                <TokenRow token="p-6" value="Card padding (CardHeader, CardContent)" />
                <TokenRow token="p-5" value="Dashboard panel cards" />
                <TokenRow token="p-4" value="Compact cards, action cards" />
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Transitions">
              <div className="flex flex-col gap-2">
                <TokenRow token="duration-300" value="All hover/state transitions (buttons, cards, borders)" />
                <TokenRow token="transition-all" value="When multiple properties change (border + bg + transform)" />
                <TokenRow token="transition-colors duration-300" value="When only color/bg changes" />
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── BUTTONS ─── */}
        <Section id="buttons" title="Buttons">
          <div className="space-y-6">
            <ComponentShowcase title="Variants">
              <div className="flex flex-wrap items-center gap-4">
                <Button>Default</Button>
                <Button variant="outline">Outline</Button>
                <Button variant="secondary">Secondary</Button>
                <Button variant="ghost">Ghost</Button>
                <Button variant="destructive">Destructive</Button>
                <Button variant="primary">Primary Action</Button>
                <Button variant="link">Link</Button>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-[11px] text-text-tertiary">
                  All buttons use <code className="text-nyx-secondary-400">rounded-lg border-white/[0.08]</code> at <code className="text-nyx-secondary-400">h-8</code> default height. <code className="text-nyx-secondary-400">variant=&quot;primary&quot;</code> is reserved for prominent form actions (save, submit). One size only — no size variants in application code.
                </p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Sizes">
              <div className="flex flex-wrap items-center gap-4">
                <Button>Default (h-8)</Button>
                <Button size="icon"><Plus className="h-4 w-4" /></Button>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-[11px] text-text-tertiary">
                  One standard height: <code className="text-nyx-secondary-400">h-8</code> (32px). Icon-only buttons use <code className="text-nyx-secondary-400">size=&quot;icon&quot;</code> (h-8 w-8). No small/large variants in application code.
                </p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="States">
              <div className="flex flex-wrap items-center gap-4">
                <Button>Normal</Button>
                <Button isLoading>Loading</Button>
                <Button disabled>Disabled</Button>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="CTA Button (Add / Create)">
              <div className="flex flex-wrap items-center gap-4">
                <AddCtaButton label="Add Service" onClick={() => toast.info("Add Service clicked")} />
                <AddCtaButton label="New Application" onClick={() => toast.info("New App clicked")} />
                <AddCtaButton label="Register Node" onClick={() => toast.info("Register Node clicked")} />
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-[11px] text-text-tertiary">
                  All &quot;+&quot; CTA buttons use <code className="text-nyx-secondary-400">AddCtaButton</code> at <code className="text-nyx-secondary-400">h-8 rounded-lg</code>. Icon in a <code className="text-nyx-secondary-400">rounded-[6px]</code> bordered container.
                </p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="With Icons (Bordered Container)">
              <div className="flex flex-wrap items-center gap-3">
                <Button><ButtonIcon><Search className="h-3 w-3" /></ButtonIcon> Search</Button>
                <Button variant="outline"><ButtonIcon><Settings className="h-3 w-3" /></ButtonIcon> Settings</Button>
                <Button variant="destructive"><ButtonIcon variant="destructive"><Trash2 className="h-3 w-3" /></ButtonIcon> Delete</Button>
                <Button variant="primary"><ButtonIcon variant="primary"><Search className="h-3 w-3" /></ButtonIcon> Get app</Button>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-[11px] text-text-tertiary">
                  Icons use <code className="text-nyx-secondary-400">ButtonIcon</code> wrapper: <code className="text-nyx-secondary-400">rounded-[6px] border-white/[0.08] bg-white/[0.04]</code>. Destructive variant uses <code className="text-nyx-secondary-400">border-destructive/20 bg-destructive/10</code>. Primary/gradient variant uses <code className="text-nyx-secondary-400">border-white/20 bg-white/10</code>.
                </p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── INPUTS ─── */}
        <Section id="inputs" title="Inputs">
          <div className="space-y-6">
            <ComponentShowcase title="Text Input">
              <div className="max-w-md space-y-4">
                <div>
                  <Label>Default</Label>
                  <Input placeholder="Enter value..." />
                </div>
                <div>
                  <Label>Disabled</Label>
                  <Input placeholder="Disabled input" disabled />
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Select">
              <div className="max-w-md">
                <Select>
                  <SelectTrigger>
                    <SelectValue placeholder="Select option..." />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="a">Option A</SelectItem>
                    <SelectItem value="b">Option B</SelectItem>
                    <SelectItem value="c">Option C</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Input Specs">
              <div className="rounded-lg bg-muted/50 p-3">
                <p className="text-[11px] text-text-tertiary">
                  Height: <code className="text-nyx-secondary-400">h-10</code> | Radius: <code className="text-nyx-secondary-400">rounded-lg</code> | Font: <code className="text-nyx-secondary-400">text-[13px]</code> | Placeholder: <code className="text-nyx-secondary-400">text-text-tertiary</code> | Focus: <code className="text-nyx-secondary-400">ring-2 ring-ring</code>
                </p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── CARDS ─── */}
        <Section id="cards" title="Cards">
          <div className="space-y-6">
            <ComponentShowcase title="Standard Card">
              <Card>
                <CardHeader>
                  <CardTitle>Card Title</CardTitle>
                  <CardDescription>A description of the card content.</CardDescription>
                </CardHeader>
                <CardContent>
                  <p className="text-[12px]">Card body content goes here.</p>
                </CardContent>
              </Card>
            </ComponentShowcase>

            <ComponentShowcase title="Dashboard Panel Card">
              <div className="grid gap-4 sm:grid-cols-2">
                {/* Security Posture style */}
                <div className="rounded-xl border border-border/50 bg-card p-5 flex flex-col gap-4">
                  <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">Security Posture</p>
                  <div className="flex items-center gap-4">
                    <div className="shrink-0 flex h-12 w-12 items-center justify-center rounded-full border-2 border-nyx-500/40">
                      <span className="text-[14px] font-bold nyx-gradient-text">80%</span>
                    </div>
                    <div className="flex flex-col gap-0.5">
                      <span className="text-[20px] font-bold nyx-gradient-text leading-none" style={{ letterSpacing: "-0.02em" }}>80%</span>
                      <span className="text-[11px] text-text-tertiary">security score</span>
                    </div>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium bg-nyx-500/10 text-nyx-secondary-400 border border-nyx-500/20">
                      <CheckCircle2 className="h-3 w-3 shrink-0" /> Email
                    </div>
                    <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium bg-nyx-500/10 text-nyx-secondary-400 border border-nyx-500/20">
                      <CheckCircle2 className="h-3 w-3 shrink-0" /> MFA
                    </div>
                    <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium text-text-tertiary border border-dashed border-border/40">
                      <div className="h-1.5 w-1.5 shrink-0 rounded-full bg-border/60" /> Keys
                    </div>
                  </div>
                </div>
                {/* Quick Links style */}
                <div className="rounded-xl border border-border/50 bg-card p-5 flex flex-col gap-3">
                  <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">Quick Links</p>
                  <div className="flex flex-col gap-1.5">
                    {["Documentation", "AI Setup Guide", "Integration Guide"].map((label) => (
                      <div key={label} className="flex items-center justify-between rounded-lg px-2 py-1.5 -mx-2 text-[12px] text-muted-foreground transition-colors duration-300 hover:bg-white/[0.03] hover:text-foreground cursor-pointer">
                        {label}
                        <ArrowRight className="h-3 w-3 text-text-tertiary" />
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Stat Card">
              <div className="grid grid-cols-2 lg:grid-cols-3 gap-3">
                {[
                  { label: "Services", value: "12", icon: <Server className="h-3.5 w-3.5" />, footer: "Registered services" },
                  { label: "API Keys", value: "4", icon: <KeyRound className="h-3.5 w-3.5" />, footer: "Active keys" },
                  { label: "MFA Status", value: "Enabled", icon: <ShieldCheck className="h-3.5 w-3.5" />, footer: "Multi-factor authentication active", success: true },
                ].map((s) => (
                  <div key={s.label} className="group flex flex-col gap-3 rounded-xl border border-border/50 bg-card p-4 transition-all duration-300 hover:border-white/[0.15] cursor-pointer">
                    <div className="flex flex-col gap-1.5">
                      <span className="text-text-tertiary transition-transform duration-300 group-hover:-translate-y-0.5">{s.icon}</span>
                      <span className="text-[11px] font-semibold uppercase tracking-wider text-text-tertiary">{s.label}</span>
                    </div>
                    <span className={cn("text-[24px] font-bold leading-none", s.success ? "text-success" : "text-foreground")} style={{ letterSpacing: "-0.02em" }}>
                      {s.value}
                    </span>
                    <span className="text-[11px] text-text-tertiary">{s.footer}</span>
                  </div>
                ))}
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Action Card">
              <div className="grid gap-3 sm:grid-cols-3">
                {[
                  { icon: <Plus className="h-4 w-4" />, title: "Primary action", desc: "With gradient icon container.", primary: true },
                  { icon: <ShieldCheck className="h-4 w-4" />, title: "Secondary action", desc: "With bordered icon container.", primary: false },
                  { icon: <Zap className="h-4 w-4" />, title: "Another action", desc: "Consistent hover behavior.", primary: false },
                ].map((c) => (
                  <div key={c.title} className={cn("group flex flex-col gap-3 rounded-xl border p-5 transition-all duration-300 cursor-pointer", c.primary ? "border-nyx-500/20 bg-nyx-500/[0.04] hover:border-white/[0.15] hover:bg-nyx-500/[0.06]" : "border-border/50 bg-card hover:border-white/[0.15]")}>
                    <div className={cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] transition-all duration-300 group-hover:-translate-y-0.5", c.primary ? "nyx-gradient-vivid text-white" : "border border-white/[0.08] bg-white/[0.04] text-text-tertiary group-hover:text-foreground group-hover:border-white/[0.15]")}>
                      {c.icon}
                    </div>
                    <div>
                      <p className="text-[14px] font-semibold text-foreground">{c.title}</p>
                      <p className="text-[12px] text-muted-foreground mt-0.5">{c.desc}</p>
                    </div>
                  </div>
                ))}
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Card Hover Rules">
              <div className="rounded-lg bg-muted/50 p-3">
                <p className="text-[11px] text-text-tertiary">
                  All interactive cards: <code className="text-nyx-secondary-400">transition-all duration-300</code>. Hover border: <code className="text-nyx-secondary-400">border-white/[0.15]</code> (neutral) or <code className="text-nyx-secondary-400">border-nyx-500/40</code> (brand). Icon: <code className="text-nyx-secondary-400">group-hover:-translate-y-0.5</code>.
                </p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── BADGES ─── */}
        <Section id="badges" title="Badges">
          <ComponentShowcase title="Variants">
            <div className="flex flex-wrap gap-3">
              <Badge>Default</Badge>
              <Badge variant="secondary">Secondary</Badge>
              <Badge variant="destructive">Destructive</Badge>
              <Badge variant="success">Success</Badge>
              <Badge variant="warning">Warning</Badge>
              <Badge variant="info">Info</Badge>
              <Badge variant="accent">Accent</Badge>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── TABS ─── */}
        <Section id="tabs" title="Tabs">
          <ComponentShowcase title="Default Tabs">
            <Tabs defaultValue="one">
              <TabsList>
                <TabsTrigger value="one">First Tab</TabsTrigger>
                <TabsTrigger value="two">Second Tab</TabsTrigger>
                <TabsTrigger value="three">Third Tab</TabsTrigger>
              </TabsList>
              <TabsContent value="one">
                <p className="text-[12px] text-muted-foreground pt-2">Content for the first tab.</p>
              </TabsContent>
              <TabsContent value="two">
                <p className="text-[12px] text-muted-foreground pt-2">Content for the second tab.</p>
              </TabsContent>
              <TabsContent value="three">
                <p className="text-[12px] text-muted-foreground pt-2">Content for the third tab.</p>
              </TabsContent>
            </Tabs>
          </ComponentShowcase>
        </Section>

        {/* ─── TABLES ─── */}
        <Section id="tables" title="Tables">
          <ComponentShowcase title="Standard Table">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow>
                  <TableCell className="font-medium">openai-prod</TableCell>
                  <TableCell><Badge variant="success">Active</Badge></TableCell>
                  <TableCell>API Key</TableCell>
                  <TableCell className="text-right">
                    <Button variant="ghost" size="icon"><Edit className="h-3.5 w-3.5" /></Button>
                    <Button variant="ghost" size="icon"><Trash2 className="h-3.5 w-3.5 text-destructive" /></Button>
                  </TableCell>
                </TableRow>
                <TableRow>
                  <TableCell className="font-medium">anthropic-dev</TableCell>
                  <TableCell><Badge variant="warning">Expiring</Badge></TableCell>
                  <TableCell>OAuth</TableCell>
                  <TableCell className="text-right">
                    <Button variant="ghost" size="icon"><Edit className="h-3.5 w-3.5" /></Button>
                    <Button variant="ghost" size="icon"><Trash2 className="h-3.5 w-3.5 text-destructive" /></Button>
                  </TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </ComponentShowcase>
        </Section>

        {/* ─── DROPDOWNS ─── */}
        <Section id="dropdowns" title="Dropdowns">
          <ComponentShowcase title="Dropdown Menu">
            <div className="flex gap-4">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="outline">
                    Options <ChevronDown className="ml-2 h-3.5 w-3.5" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent className="p-2">
                  <DropdownMenuItem className="rounded-lg"><Edit className="mr-2" /> Edit</DropdownMenuItem>
                  <DropdownMenuItem className="rounded-lg"><Copy className="mr-2" /> Duplicate</DropdownMenuItem>
                  <DropdownMenuItem className="rounded-lg text-destructive focus:text-destructive [&_svg]:text-destructive"><Trash2 className="mr-2" /> Delete</DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            <div className="mt-4 rounded-lg bg-muted/50 p-3">
              <p className="text-[11px] text-text-tertiary">
                Container: <code className="text-nyx-secondary-400">p-2 rounded-xl</code>. Items: <code className="text-nyx-secondary-400">rounded-lg px-2 py-2</code>. No separators — use spacing instead.
              </p>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── DIALOGS ─── */}
        <Section id="dialogs" title="Dialogs">
          <ComponentShowcase title="Standard Dialog">
            <Dialog>
              <DialogTrigger asChild>
                <Button variant="outline">Open Dialog</Button>
              </DialogTrigger>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>Dialog Title</DialogTitle>
                  <DialogDescription>A brief description of what this dialog does.</DialogDescription>
                </DialogHeader>
                <div className="space-y-4">
                  <div className="space-y-2">
                    <Label>Name</Label>
                    <Input placeholder="Enter name..." />
                  </div>
                </div>
                <DialogFooter>
                  <Button variant="ghost">Cancel</Button>
                  <Button>Save</Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
            <div className="mt-4 rounded-lg bg-muted/50 p-3">
              <p className="text-[11px] text-text-tertiary">
                Padding: <code className="text-nyx-secondary-400">p-6</code>. Radius: <code className="text-nyx-secondary-400">rounded-xl</code>. Gap: <code className="text-nyx-secondary-400">gap-6</code>. Overlay: <code className="text-nyx-secondary-400">bg-black/60 backdrop-blur-sm</code>.
              </p>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── TOOLTIPS ─── */}
        <Section id="tooltips" title="Tooltips">
          <ComponentShowcase title="Standard Tooltip">
            <div className="flex gap-4">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="outline">Hover me</Button>
                </TooltipTrigger>
                <TooltipContent>Tooltip content</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon"><Info className="h-4 w-4" /></Button>
                </TooltipTrigger>
                <TooltipContent>Information tooltip</TooltipContent>
              </Tooltip>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── SWITCHES ─── */}
        <Section id="switches" title="Switches">
          <ComponentShowcase title="Switch States">
            <div className="flex items-center gap-8">
              <div className="flex items-center gap-3">
                <Switch checked={switchVal} onCheckedChange={setSwitchVal} />
                <Label>Enabled</Label>
              </div>
              <div className="flex items-center gap-3">
                <Switch checked={false} disabled />
                <Label className="text-text-tertiary">Disabled</Label>
              </div>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── SKELETONS ─── */}
        <Section id="skeletons" title="Skeletons">
          <ComponentShowcase title="Loading States">
            <div className="space-y-4">
              <div className="space-y-2">
                <Skeleton className="h-10 w-48" />
                <Skeleton className="h-32 w-full" />
              </div>
              <div className="grid grid-cols-4 gap-3">
                <Skeleton className="h-20 w-full rounded-xl" />
                <Skeleton className="h-20 w-full rounded-xl" />
                <Skeleton className="h-20 w-full rounded-xl" />
                <Skeleton className="h-20 w-full rounded-xl" />
              </div>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── ICON STYLING ─── */}
        <Section id="icons" title="Icon Styling">
          <div className="space-y-6">
            <ComponentShowcase title="Icon Container Variants">
              <div className="flex flex-wrap gap-6 items-center">
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-8 w-8 items-center justify-center rounded-[8px] nyx-gradient-vivid text-white">
                    <Plus className="h-4 w-4" />
                  </div>
                  <span className="text-[10px] text-text-tertiary">Gradient</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-white/[0.08] bg-white/[0.04] text-text-tertiary">
                    <Settings className="h-4 w-4" />
                  </div>
                  <span className="text-[10px] text-text-tertiary">Bordered</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-nyx-500/30 bg-nyx-500/15 text-nyx-secondary-400">
                    <Zap className="h-4 w-4" />
                  </div>
                  <span className="text-[10px] text-text-tertiary">Brand bordered</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] border border-white/[0.08] bg-white/[0.04] text-text-tertiary">
                    <Plus className="h-3 w-3" />
                  </div>
                  <span className="text-[10px] text-text-tertiary">Small (CTA)</span>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Icon Sizes">
              <div className="flex flex-wrap gap-6 items-end">
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-3 w-3 text-text-tertiary" />
                  <code className="text-[10px] font-mono text-text-tertiary">12px</code>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-3.5 w-3.5 text-text-tertiary" />
                  <code className="text-[10px] font-mono text-text-tertiary">14px</code>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-4 w-4 text-text-tertiary" />
                  <code className="text-[10px] font-mono text-text-tertiary">16px</code>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-[18px] w-[18px] text-text-tertiary" />
                  <code className="text-[10px] font-mono text-text-tertiary">18px</code>
                </div>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-[11px] text-text-tertiary">
                  Nav sidebar: <code className="text-nyx-secondary-400">16px</code>. Top bar chrome: <code className="text-nyx-secondary-400">18px</code>. Inline buttons: <code className="text-nyx-secondary-400">14px</code>. Stat cards: <code className="text-nyx-secondary-400">14px</code>. Tiny indicators: <code className="text-nyx-secondary-400">12px</code>.
                </p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── TOASTS ─── */}
        <Section id="toasts" title="Toasts">
          <ComponentShowcase title="Toast Variants">
            <div className="flex flex-col gap-3">
              <div className="flex items-center gap-3 rounded-xl border border-success/30 bg-success/[0.06] p-3 text-[13px] text-success">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-success/20 bg-success/10">
                  <Check className="h-3 w-3" />
                </span>
                <span>Action completed successfully</span>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-destructive/30 bg-destructive/[0.06] p-3 text-[13px] text-destructive">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-destructive/20 bg-destructive/10">
                  <X className="h-3 w-3" />
                </span>
                <span>Something went wrong</span>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-nyx-500/30 bg-nyx-500/[0.06] p-3 text-[13px] text-nyx-secondary-400">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-nyx-500/20 bg-nyx-500/10">
                  <Info className="h-3 w-3" />
                </span>
                <span>Here is some information</span>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-warning/30 bg-warning/[0.06] p-3 text-[13px] text-warning">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-warning/20 bg-warning/10">
                  <AlertCircle className="h-3 w-3" />
                </span>
                <span>Please review this</span>
              </div>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── PATTERNS ─── */}
        <Section id="patterns" title="Patterns">
          <div className="space-y-6">
            <ComponentShowcase title="Section Overline + Content">
              <div className="flex flex-col gap-3">
                <p className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">Section Label</p>
                <p className="text-[12px] text-muted-foreground">Content below the overline label.</p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Status Banner (All Clear)">
              <div className="flex items-center gap-3 rounded-xl border border-success/15 bg-success/[0.04] px-4 py-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-success/10">
                  <ShieldCheck className="h-4.5 w-4.5 text-success" />
                </div>
                <div>
                  <p className="text-[13px] font-semibold text-foreground">All systems clear</p>
                  <p className="text-[11px] text-muted-foreground">Nothing needs your attention right now.</p>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Status Banner (Error)">
              <ErrorBanner message="Failed to load services" onRetry={() => {}} />
            </ComponentShowcase>

            <ComponentShowcase title="Attention Row">
              <div className="flex flex-col gap-2">
                <div className="group flex items-center gap-3 rounded-lg px-3 py-2 transition-colors duration-300 hover:bg-white/[0.03] cursor-pointer">
                  <div className="h-1.5 w-1.5 rounded-full shrink-0 bg-warning" />
                  <span className="shrink-0 text-warning"><ShieldAlert className="h-4 w-4" /></span>
                  <span className="text-[13px] text-foreground flex-1">3 pending approvals</span>
                  <ArrowRight className="h-3 w-3 text-text-tertiary opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
                <div className="group flex items-center gap-3 rounded-lg px-3 py-2 transition-colors duration-300 hover:bg-white/[0.03] cursor-pointer">
                  <div className="h-1.5 w-1.5 rounded-full shrink-0 bg-error" />
                  <span className="shrink-0 text-error"><AlertCircle className="h-4 w-4" /></span>
                  <span className="text-[13px] text-foreground flex-1">1 node offline</span>
                  <ArrowRight className="h-3 w-3 text-text-tertiary opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Status Chips">
              <div className="flex flex-wrap gap-2">
                <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium bg-nyx-500/10 text-nyx-secondary-400 border border-nyx-500/20">
                  <CheckCircle2 className="h-3 w-3 shrink-0" /> Complete
                </div>
                <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium text-text-tertiary border border-dashed border-border/40">
                  <div className="h-1.5 w-1.5 shrink-0 rounded-full bg-border/60" /> Incomplete
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title="Empty State">
              <div className="flex flex-col items-center gap-4 py-12">
                <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
                  <Cable className="h-6 w-6 text-muted-foreground" />
                </div>
                <div className="text-center max-w-md space-y-1">
                  <p className="text-[12px] font-medium text-muted-foreground/30">No items yet</p>
                  <p className="text-[13px] text-muted-foreground">Get started by adding your first item.</p>
                </div>
                <AddCtaButton label="Add Item" onClick={() => {}} />
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ── UX Rules ── */}
        <Section id="ux-rules" title="UX Rules">
          <div className="space-y-6">
            <div className="space-y-4">
              {[
                {
                  rule: "CTA buttons disabled when no changes",
                  detail: "Save, Create, and Submit buttons must be disabled when no changes have been made or required fields are empty. Never present an active CTA that would submit unchanged data.",
                  code: "disabled={isPending || !hasChanges}",
                },
                {
                  rule: "Standalone action buttons top-right of card",
                  detail: "Standalone action buttons (Rotate Credential, Deactivate, Edit Scope, Route via Node, etc.) sit top-right in the card header, inline with the card title. Form save/cancel pairs stay bottom-right.",
                  code: '<div className="flex items-center justify-between">\n  <CardTitle>Title</CardTitle>\n  <Button>Action</Button>\n</div>',
                },
                {
                  rule: "Form save/cancel buttons bottom-right",
                  detail: "Save/Cancel button pairs in edit forms must be right-aligned at the bottom of their container. Use flex justify-end gap-2.",
                  code: 'className="flex justify-end gap-2"',
                },
                {
                  rule: "Section titles use consistent typography",
                  detail: "All section titles above card grids must use the same pattern: an icon (h-4 w-4, text-muted-foreground) followed by text-[13px] font-semibold text-foreground. Never mix uppercase tertiary text with this pattern on the same page.",
                  code: '<Icon className="h-4 w-4 text-muted-foreground" />\n<h3 className="text-[13px] font-semibold text-foreground">Title</h3>',
                },
                {
                  rule: "Create/Add buttons top-right of section",
                  detail: "When a card section has a create or add action, the button sits top-right of the section header row, inline with the section title. Never place create buttons in their own standalone row.",
                  code: '<div className="flex items-center justify-between">\n  <div className="flex items-center gap-2">...</div>\n  <Button>Create</Button>\n</div>',
                },
                {
                  rule: "Trash icons always red",
                  detail: "All Trash2 / delete icons must use text-destructive. Never use text-muted-foreground or any other color for delete actions.",
                  code: '<Trash2 className="h-3.5 w-3.5 text-destructive" />',
                },
                {
                  rule: "Icon containers use rounded-xl, never rounded-full",
                  detail: "All icon containers in empty states, cards, and list items must use rounded-xl (rounded square). The rounded-full (circle) shape is never used for icon containers.",
                  code: '"flex h-14 w-14 items-center justify-center rounded-xl border border-border"',
                },
                {
                  rule: "Card hover borders are grey, not purple",
                  detail: "Hovering over a card increases the white border opacity. Never use purple/primary color for card hover borders.",
                  code: '"hover:border-white/[0.15] hover:bg-accent/30"',
                },
                {
                  rule: "Clickable rows get hover state, non-clickable rows do not",
                  detail: "Table rows that navigate on click use cursor-pointer and a subtle hover background. Non-interactive rows have no hover effect.",
                  code: '"cursor-pointer hover:bg-white/[0.03]"',
                },
                {
                  rule: "Toast semantics must match action",
                  detail: "Use toast.success for enabling/creating. Use toast.warning for restricting/disabling. Use toast.error for failures. Never use success for restrictive actions.",
                  code: 'toast.warning("Agent restricted to bound services only")',
                },
                {
                  rule: "Truncate text to prevent overlap",
                  detail: "Long text (API key prefixes, URLs, names) must be truncated to prevent overlapping adjacent content. Use the truncate class with min-w-0 on the container.",
                  code: '"block truncate rounded bg-muted px-1.5 py-0.5"',
                },
                {
                  rule: "Table headers are uppercase micro text",
                  detail: "All table headers use 11px, font-semibold, uppercase, tracking-[1.5px], text-text-tertiary. This is set in the base TableHead component.",
                  code: '"text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary"',
                },
                {
                  rule: "Table wrappers use consistent card styling",
                  detail: "All tables must be wrapped in a container with rounded-xl, border, bg-card, and overflow-hidden.",
                  code: '"rounded-xl border border-border/50 bg-card overflow-hidden"',
                },
                {
                  rule: "Titles must always be in title case",
                  detail: "All section titles, page titles, card titles, tab labels, and headings must use Title Case (capitalize the first letter of each major word). Never use sentence case for titles.",
                  code: '"My Services" not "My services"',
                },
                {
                  rule: "Table cell alignment is always top",
                  detail: "All table cells use align-top (set in the base TableCell component). This ensures multi-line content in any column stays readable. Never override to align-middle.",
                  code: '"px-4 py-4 align-top text-[13px] text-foreground"',
                },
                {
                  rule: "Table cell text size is consistent at 13px",
                  detail: "All primary table cell content uses text-[13px] (set in the base TableCell). Secondary metadata can use text-[11px] or text-[10px]. Never use text-sm (14px) or text-xs (12px) for primary content.",
                  code: 'Base: text-[13px] | Secondary: text-[11px] text-muted-foreground',
                },
                {
                  rule: "Clickable rows have hover state, not hyperlinks",
                  detail: "When a table row navigates on click, the entire row gets cursor-pointer and hover:bg-white/[0.03]. The name/title cell should NOT be styled as a hyperlink (no underline, no link color). Non-clickable rows have no hover effect.",
                  code: '<TableRow className="cursor-pointer hover:bg-white/[0.03]" onClick={navigate}>',
                },
                {
                  rule: "Error/not-found states use PageHeader + ErrorBanner",
                  detail: "When a page fails to load or an entity is not found, show PageHeader with a 'Not Found' title and ErrorBanner with the error message and a retry button. Never use centered icon + text layouts for error states.",
                  code: '<PageHeader title="Entity Not Found" />\n<ErrorBanner message={error?.message ?? "Not found."} onRetry={refetch} />',
                },
                {
                  rule: "Button hierarchy: large CTAs use ButtonIcon, minor edits use icon-only",
                  detail: "Large CTA actions (Deactivate, Route via Node, Rotate Credential, Delete, Add Service) always use ButtonIcon with icon + text. Minor edit actions (Edit Scope, Edit Headers, Edit Limits, Edit WS Frames) use a ghost pencil icon-only button.",
                  code: 'CTA: <Button><ButtonIcon><Icon /></ButtonIcon>Label</Button>\nEdit: <Button size="icon" variant="ghost"><Pencil /></Button>',
                },
              ].map((item) => (
                <div key={item.rule} className="rounded-xl border border-border/50 bg-card p-5 space-y-2">
                  <p className="text-[13px] font-semibold text-foreground">{item.rule}</p>
                  <p className="text-[12px] text-muted-foreground">{item.detail}</p>
                  <code className="block text-[11px] font-mono text-nyx-secondary-400 bg-muted rounded-lg px-3 py-2">
                    {item.code}
                  </code>
                </div>
              ))}
            </div>
          </div>
        </Section>
      </div>
    </div>
    </div>
  );
}
