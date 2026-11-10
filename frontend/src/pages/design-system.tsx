import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

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
import { PlatformIcon, PLATFORM_ICON_OPTIONS } from "@/components/platform-icon";
import { ServiceIcon, type ServiceIconSize } from "@/components/service-icon";
import { CUSTOM_SERVICE_SLUGS, SPEC_CATALOG_SLUGS } from "@/components/service-icons";
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
      <h2 className="text-24 font-bold tracking-tight mb-6" style={{ letterSpacing: "-0.02em" }}>
        {title}
      </h2>
      {children}
    </section>
  );
}

function Swatch({ name, cls, textCls }: { readonly name: string; readonly cls: string; readonly textCls?: string }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className={cn("h-12 w-full rounded-lg border border-hairline", cls)} />
      <span className={cn("text-11 font-mono", textCls ?? "text-text-tertiary")}>{name}</span>
    </div>
  );
}

function TokenRow({ token, value, preview }: { readonly token: string; readonly value: string; readonly preview?: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between py-2 border-b border-border/20 last:border-0">
      <code className="text-12 font-mono text-nyx-secondary-400">{token}</code>
      <div className="flex items-center gap-3">
        {preview}
        <span className="text-12 text-muted-foreground">{value}</span>
      </div>
    </div>
  );
}

function ComponentShowcase({ title, children }: { readonly title: string; readonly children: React.ReactNode }) {
  return (
    <div className="space-y-3">
      <p className="text-11 font-semibold uppercase tracking-[1.5px] text-text-tertiary">{title}</p>
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

const ICON_SIZE_OPTIONS: readonly {
  value: ServiceIconSize;
  label: string;
}[] = [
  { value: "2xs", get label() { return translateUi("2xs"); } },
  { value: "xs", get label() { return translateUi("xs"); } },
  { value: "sm", get label() { return translateUi("sm"); } },
  { value: "md", get label() { return translateUi("md"); } },
  { value: "lg", get label() { return translateUi("lg"); } },
  { value: "xl", get label() { return translateUi("xl"); } },
];

const ICON_AUDIT_SLUGS = [...SPEC_CATALOG_SLUGS, ...CUSTOM_SERVICE_SLUGS];

function iconAuditLabel(slug: string) {
  return slug
    .replace(/^(api|llm)-/, "")
    .split("-")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

export function DesignSystemPage() {
  const { t } = useUiTranslation();
  const [switchVal, setSwitchVal] = useState(true);
  const [iconSearch, setIconSearch] = useState("");
  const normalizedIconSearch = iconSearch.trim().toLowerCase();
  const filteredIconAuditSlugs = ICON_AUDIT_SLUGS.filter(
    (slug) =>
      !normalizedIconSearch ||
      slug.toLowerCase().includes(normalizedIconSearch) ||
      iconAuditLabel(slug).toLowerCase().includes(normalizedIconSearch),
  );

  return (
    <div className="min-h-dvh bg-background text-foreground">
    <div className="mx-auto max-w-[1200px] flex gap-10 px-8 py-10">
      {/* Sticky nav */}
      <nav className="hidden xl:flex shrink-0 w-[180px] flex-col gap-1 sticky top-10 self-start">
        <p className="text-11 font-semibold uppercase tracking-[1.5px] text-text-tertiary mb-2">{t("Components")}</p>
        {DS_NAV.map((item) => (
          <a
            key={item.id}
            href={`#${item.id}`}
            className="text-13 text-muted-foreground hover:text-foreground transition-colors duration-300 py-1"
          >
            {item.label}
          </a>
        ))}
      </nav>

      {/* Main content */}
      <div className="flex-1 min-w-0 space-y-16">
        {/* Header */}
        <div>
          <p className="text-11 font-semibold uppercase tracking-[1.5px] text-nyx-secondary-400 mb-2">{t("Design System")}</p>
          <h1 className="text-28 font-bold leading-[1.1]" style={{ letterSpacing: "-0.03em" }}>{t("NyxID Components")}</h1>
          <p className="text-12 text-muted-foreground mt-2">{t("The canonical reference for every UI primitive. All components in the product must follow these patterns.")}</p>
        </div>

        {/* ─── COLORS ─── */}
        <Section id="colors" title={t("Colors")}>
          <div className="space-y-6">
            <ComponentShowcase title={t("Brand Primary")}>
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

            <ComponentShowcase title={t("Brand Secondary")}>
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

            <ComponentShowcase title={t("Semantic")}>
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

            <ComponentShowcase title={t("Border Opacity Scale")}>
              <div className="flex flex-col gap-2">
                <TokenRow token="border-hairline" value="Passive — default borders for chrome elements" preview={<div className="h-8 w-24 rounded-lg border border-hairline" />} />
                <TokenRow token="border-hairline-strong" value="Hover — elevated state on interactive borders" preview={<div className="h-8 w-24 rounded-lg border border-hairline-strong" />} />
                <TokenRow token="border-border/50" value="Cards — subtle card boundaries" preview={<div className="h-8 w-24 rounded-lg border border-border/50 bg-card" />} />
                <TokenRow token="border-border/30" value="Layout — sidebar & header dividers" preview={<div className="h-8 w-24 rounded-lg border border-border/30" />} />
                <TokenRow token="border-nyx-500/20" value="Accent — primary brand cards" preview={<div className="h-8 w-24 rounded-lg border border-nyx-500/20 bg-nyx-500/[0.04]" />} />
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── TYPOGRAPHY ─── */}
        <Section id="typography" title={t("Typography")}>
          <ComponentShowcase title={t("Scale")}>
            <div className="space-y-6">
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Page Title — 48px / bold / -0.035em")}</p>
                <h1 className="text-28 font-bold leading-[1.1]" style={{ letterSpacing: "-0.03em" }}>{t("Page Title")}</h1>
              </div>
              <Separator />
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Section Title — 24px / bold / -0.02em")}</p>
                <h2 className="text-24 font-bold" style={{ letterSpacing: "-0.02em" }}>{t("Section Title")}</h2>
              </div>
              <Separator />
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Card Title — 16px (base) / semibold")}</p>
                <h3 className="text-base font-semibold">{t("Card Title")}</h3>
              </div>
              <Separator />
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Body — 14px (text-sm) / normal")}</p>
                <p className="text-sm">{t("Body text for descriptions and content paragraphs.")}</p>
              </div>
              <Separator />
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Secondary — 13px / normal / text-muted-foreground")}</p>
                <p className="text-13 text-muted-foreground">{t("Secondary text for compact descriptions and dashboard UI.")}</p>
              </div>
              <Separator />
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Small — 12px / normal")}</p>
                <p className="text-12">{t("Small text for table cells, info rows, and metadata.")}</p>
              </div>
              <Separator />
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Overline — 11px / semibold / uppercase / tracking-[1.5px] / text-text-tertiary")}</p>
                <p className="text-11 font-semibold uppercase tracking-[1.5px] text-text-tertiary">{t("Section Overline")}</p>
              </div>
              <Separator />
              <div>
                <p className="text-11 text-text-tertiary mb-1">{t("Caption — 10px / medium — badges only")}</p>
                <p className="text-10 font-medium">{t("Badge caption text")}</p>
              </div>
            </div>
          </ComponentShowcase>

          <div className="mt-6">
            <ComponentShowcase title={t("Text Colors")}>
              <div className="space-y-2">
                <p className="text-foreground text-13"><code className="text-nyx-secondary-400 text-11 font-mono mr-3">text-foreground</code>{t(" Primary text — headings, titles, important content")}</p>
                <p className="text-muted-foreground text-13"><code className="text-nyx-secondary-400 text-11 font-mono mr-3">text-muted-foreground</code>{t(" Secondary text — descriptions, body copy")}</p>
                <p className="text-text-tertiary text-13"><code className="text-nyx-secondary-400 text-11 font-mono mr-3">text-text-tertiary</code>{t(" Tertiary text — placeholders, overlines, captions")}</p>
                <p className="text-nyx-secondary-400 text-13"><code className="text-nyx-secondary-400 text-11 font-mono mr-3">text-nyx-secondary-400</code>{t(" Accent text — links, active states")}</p>
                <p className="text-success text-13"><code className="text-nyx-secondary-400 text-11 font-mono mr-3">text-success</code>{t(" Success state")}</p>
                <p className="text-warning text-13"><code className="text-nyx-secondary-400 text-11 font-mono mr-3">text-warning</code>{t(" Warning state")}</p>
                <p className="text-destructive text-13"><code className="text-nyx-secondary-400 text-11 font-mono mr-3">text-destructive</code>{t(" Destructive / error state")}</p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── SPACING & RADIUS ─── */}
        <Section id="spacing" title={t("Spacing & Radius")}>
          <div className="space-y-6">
            <ComponentShowcase title={t("Border Radius")}>
              <div className="flex flex-wrap gap-6 items-end">
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-xl border border-border bg-muted" />
                  <code className="text-11 font-mono text-text-tertiary">rounded-xl</code>
                  <span className="text-10 text-text-tertiary">{t("Cards, dialogs, selects")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-lg border border-border bg-muted" />
                  <code className="text-11 font-mono text-text-tertiary">rounded-lg</code>
                  <span className="text-10 text-text-tertiary">{t("Buttons, inputs, items")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-[8px] border border-border bg-muted" />
                  <code className="text-11 font-mono text-text-tertiary">rounded-[8px]</code>
                  <span className="text-10 text-text-tertiary">{t("Icon containers")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-[6px] border border-border bg-muted" />
                  <code className="text-11 font-mono text-text-tertiary">rounded-[6px]</code>
                  <span className="text-10 text-text-tertiary">{t("Small elements, kbds")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-[10px] border border-border bg-muted" />
                  <code className="text-11 font-mono text-text-tertiary">rounded-[10px]</code>
                  <span className="text-10 text-text-tertiary">{t("Badges (pill)")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="h-16 w-16 rounded-full border border-border bg-muted" />
                  <code className="text-11 font-mono text-text-tertiary">rounded-full</code>
                  <span className="text-10 text-text-tertiary">{t("Status dots, chips")}</span>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Spacing Tokens")}>
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

            <ComponentShowcase title={t("Transitions")}>
              <div className="flex flex-col gap-2">
                <TokenRow token="duration-300" value="All hover/state transitions (buttons, cards, borders)" />
                <TokenRow token="transition-all" value="When multiple properties change (border + bg + transform)" />
                <TokenRow token="transition-colors duration-300" value="When only color/bg changes" />
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── BUTTONS ─── */}
        <Section id="buttons" title={t("Buttons")}>
          <div className="space-y-6">
            <ComponentShowcase title={t("Variants")}>
              <div className="flex flex-wrap items-center gap-4">
                <Button>{t("Default")}</Button>
                <Button variant="outline">{t("Outline")}</Button>
                <Button variant="secondary">{t("Secondary")}</Button>
                <Button variant="ghost">{t("Ghost")}</Button>
                <Button variant="destructive">{t("Destructive")}</Button>
                <Button variant="primary">{t("Primary Action")}</Button>
                <Button variant="link">{t("Link")}</Button>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-11 text-text-tertiary">{t("All buttons use ")}<code className="text-nyx-secondary-400">rounded-lg border-hairline</code>{t(" at ")}<code className="text-nyx-secondary-400">h-8</code>{t(" default height. ")}<code className="text-nyx-secondary-400">variant=&quot;primary&quot;</code>{t(" is reserved for prominent form actions (save, submit). One size only — no size variants in application code.")}</p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Sizes")}>
              <div className="flex flex-wrap items-center gap-4">
                <Button>{t("Default (h-8)")}</Button>
                <Button size="icon"><Plus className="h-4 w-4" /></Button>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-11 text-text-tertiary">{t("One standard height: ")}<code className="text-nyx-secondary-400">h-8</code>{t(" (32px). Icon-only buttons use ")}<code className="text-nyx-secondary-400">size=&quot;icon&quot;</code>{t(" (h-8 w-8). No small/large variants in application code.")}</p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("States")}>
              <div className="flex flex-wrap items-center gap-4">
                <Button>{t("Normal")}</Button>
                <Button isLoading>{t("Loading")}</Button>
                <Button disabled>{t("Disabled")}</Button>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("CTA Button (Add / Create)")}>
              <div className="flex flex-wrap items-center gap-4">
                <AddCtaButton label={t("Add Service")} onClick={() => toast.info(translateUi("Add Service clicked"))} />
                <AddCtaButton label={t("New Application")} onClick={() => toast.info(translateUi("New App clicked"))} />
                <AddCtaButton label={t("Register Node")} onClick={() => toast.info(translateUi("Register Node clicked"))} />
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-11 text-text-tertiary">{t("All \"+\" CTA buttons use ")}<code className="text-nyx-secondary-400">AddCtaButton</code>{t(" at ")}<code className="text-nyx-secondary-400">h-8 rounded-lg</code>{t(". Icon in a ")}<code className="text-nyx-secondary-400">rounded-[6px]</code>{t(" bordered container.")}</p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("With Icons (Bordered Container)")}>
              <div className="flex flex-wrap items-center gap-3">
                <Button><ButtonIcon><Search className="h-3 w-3" /></ButtonIcon>{t(" Search")}</Button>
                <Button variant="outline"><ButtonIcon><Settings className="h-3 w-3" /></ButtonIcon>{t(" Settings")}</Button>
                <Button variant="destructive"><ButtonIcon variant="destructive"><Trash2 className="h-3 w-3" /></ButtonIcon>{t(" Delete")}</Button>
                <Button variant="primary"><ButtonIcon variant="primary"><Search className="h-3 w-3" /></ButtonIcon>{t(" Get app")}</Button>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-11 text-text-tertiary">{t("Icons use ")}<code className="text-nyx-secondary-400">ButtonIcon</code>{t(" wrapper: ")}<code className="text-nyx-secondary-400">rounded-[6px] border-hairline bg-overlay</code>{t(". Destructive variant uses ")}<code className="text-nyx-secondary-400">border-destructive/20 bg-destructive/10</code>{t(". Primary/gradient variant uses ")}<code className="text-nyx-secondary-400">border-white/20 bg-white/10</code>.
                </p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── INPUTS ─── */}
        <Section id="inputs" title={t("Inputs")}>
          <div className="space-y-6">
            <ComponentShowcase title={t("Text Input")}>
              <div className="max-w-md space-y-4">
                <div>
                  <Label>{t("Default")}</Label>
                  <Input placeholder={t("Enter value...")} />
                </div>
                <div>
                  <Label>{t("Disabled")}</Label>
                  <Input placeholder={t("Disabled input")} disabled />
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Select")}>
              <div className="max-w-md">
                <Select>
                  <SelectTrigger>
                    <SelectValue placeholder={t("Select option...")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="a">{t("Option A")}</SelectItem>
                    <SelectItem value="b">{t("Option B")}</SelectItem>
                    <SelectItem value="c">{t("Option C")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Input Specs")}>
              <div className="rounded-lg bg-muted/50 p-3">
                <p className="text-11 text-text-tertiary">{t("Height: ")}<code className="text-nyx-secondary-400">h-10</code>{t(" | Radius: ")}<code className="text-nyx-secondary-400">rounded-lg</code>{t(" | Font: ")}<code className="text-nyx-secondary-400">text-13</code>{t(" | Placeholder: ")}<code className="text-nyx-secondary-400">text-text-tertiary</code>{t(" | Focus: ")}<code className="text-nyx-secondary-400">ring-2 ring-ring</code>
                </p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── CARDS ─── */}
        <Section id="cards" title={t("Cards")}>
          <div className="space-y-6">
            <ComponentShowcase title={t("Standard Card")}>
              <Card>
                <CardHeader>
                  <CardTitle>{t("Card Title")}</CardTitle>
                  <CardDescription>{t("A description of the card content.")}</CardDescription>
                </CardHeader>
                <CardContent>
                  <p className="text-12">{t("Card body content goes here.")}</p>
                </CardContent>
              </Card>
            </ComponentShowcase>

            <ComponentShowcase title={t("Dashboard Panel Card")}>
              <div className="grid gap-4 sm:grid-cols-2">
                {/* Security Posture style */}
                <div className="rounded-xl border border-border/50 bg-card p-5 flex flex-col gap-4">
                  <p className="text-11 font-semibold uppercase tracking-[1.5px] text-text-tertiary">{t("Security Posture")}</p>
                  <div className="flex items-center gap-4">
                    <div className="shrink-0 flex h-12 w-12 items-center justify-center rounded-full border-2 border-nyx-500/40">
                      <span className="text-14 font-bold nyx-gradient-text">80%</span>
                    </div>
                    <div className="flex flex-col gap-0.5">
                      <span className="text-20 font-bold nyx-gradient-text leading-none" style={{ letterSpacing: "-0.02em" }}>80%</span>
                      <span className="text-11 text-text-tertiary">{t("security score")}</span>
                    </div>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-11 font-medium bg-nyx-500/10 text-nyx-secondary-400 border border-nyx-500/20">
                      <CheckCircle2 className="h-3 w-3 shrink-0" />{t(" Email")}</div>
                    <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-11 font-medium bg-nyx-500/10 text-nyx-secondary-400 border border-nyx-500/20">
                      <CheckCircle2 className="h-3 w-3 shrink-0" />{t(" MFA")}</div>
                    <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-11 font-medium text-text-tertiary border border-dashed border-border/40">
                      <div className="h-1.5 w-1.5 shrink-0 rounded-full bg-border/60" />{t(" Keys")}</div>
                  </div>
                </div>
                {/* Quick Links style */}
                <div className="rounded-xl border border-border/50 bg-card p-5 flex flex-col gap-3">
                  <p className="text-11 font-semibold uppercase tracking-[1.5px] text-text-tertiary">{t("Quick Links")}</p>
                  <div className="flex flex-col gap-1.5">
                    {["Documentation", "AI Setup Guide", "Integration Guide"].map((label) => (
                      <div key={label} className="flex items-center justify-between rounded-lg px-2 py-1.5 -mx-2 text-12 text-muted-foreground transition-colors duration-300 hover:bg-overlay hover:text-foreground cursor-pointer">
                        {label}
                        <ArrowRight className="h-3 w-3 text-text-tertiary" />
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Stat Card")}>
              <div className="grid grid-cols-2 lg:grid-cols-3 gap-3">
                {[
                  { label: "Services", value: "12", icon: <Server className="h-3.5 w-3.5" />, footer: "Registered services" },
                  { label: "API Keys", value: "4", icon: <KeyRound className="h-3.5 w-3.5" />, footer: "Active keys" },
                  { label: "MFA Status", value: "Enabled", icon: <ShieldCheck className="h-3.5 w-3.5" />, footer: "Multi-factor authentication active", success: true },
                ].map((s) => (
                  <div key={s.label} className="group flex flex-col gap-3 rounded-xl border border-border/50 bg-card p-4 transition-all duration-300 hover:border-hairline-strong cursor-pointer">
                    <div className="flex flex-col gap-1.5">
                      <span className="text-text-tertiary transition-transform duration-300 group-hover:-translate-y-0.5">{s.icon}</span>
                      <span className="text-11 font-semibold uppercase tracking-wider text-text-tertiary">{s.label}</span>
                    </div>
                    <span className={cn("text-24 font-bold leading-none", s.success ? "text-success" : "text-foreground")} style={{ letterSpacing: "-0.02em" }}>
                      {s.value}
                    </span>
                    <span className="text-11 text-text-tertiary">{s.footer}</span>
                  </div>
                ))}
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Action Card")}>
              <div className="grid gap-3 sm:grid-cols-3">
                {[
                  { icon: <Plus className="h-4 w-4" />, title: "Primary action", desc: "With gradient icon container.", primary: true },
                  { icon: <ShieldCheck className="h-4 w-4" />, title: "Secondary action", desc: "With bordered icon container.", primary: false },
                  { icon: <Zap className="h-4 w-4" />, title: "Another action", desc: "Consistent hover behavior.", primary: false },
                ].map((c) => (
                  <div key={c.title} className={cn("group flex flex-col gap-3 rounded-xl border p-5 transition-all duration-300 cursor-pointer", c.primary ? "border-nyx-500/20 bg-nyx-500/[0.04] hover:border-hairline-strong hover:bg-nyx-500/[0.06]" : "border-border/50 bg-card hover:border-hairline-strong")}>
                    <div className={cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] transition-all duration-300 group-hover:-translate-y-0.5", c.primary ? "nyx-gradient-vivid text-white" : "border border-hairline bg-overlay text-text-tertiary group-hover:text-foreground group-hover:border-hairline-strong")}>
                      {c.icon}
                    </div>
                    <div>
                      <p className="text-14 font-semibold text-foreground">{c.title}</p>
                      <p className="text-12 text-muted-foreground mt-0.5">{c.desc}</p>
                    </div>
                  </div>
                ))}
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Card Hover Rules")}>
              <div className="rounded-lg bg-muted/50 p-3">
                <p className="text-11 text-text-tertiary">{t("All interactive cards: ")}<code className="text-nyx-secondary-400">transition-all duration-300</code>{t(". Hover border: ")}<code className="text-nyx-secondary-400">border-hairline-strong</code>{t(" (neutral) or ")}<code className="text-nyx-secondary-400">border-nyx-500/40</code>{t(" (brand). Icon: ")}<code className="text-nyx-secondary-400">group-hover:-translate-y-0.5</code>.
                </p>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── BADGES ─── */}
        <Section id="badges" title={t("Badges")}>
          <ComponentShowcase title={t("Variants")}>
            <div className="flex flex-wrap gap-3">
              <Badge>{t("Default")}</Badge>
              <Badge variant="secondary">{t("Secondary")}</Badge>
              <Badge variant="destructive">{t("Destructive")}</Badge>
              <Badge variant="success">{t("Success")}</Badge>
              <Badge variant="warning">{t("Warning")}</Badge>
              <Badge variant="info">{t("Info")}</Badge>
              <Badge variant="accent">{t("Accent")}</Badge>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── TABS ─── */}
        <Section id="tabs" title={t("Tabs")}>
          <ComponentShowcase title={t("Default Tabs")}>
            <Tabs defaultValue="one">
              <TabsList>
                <TabsTrigger value="one">{t("First Tab")}</TabsTrigger>
                <TabsTrigger value="two">{t("Second Tab")}</TabsTrigger>
                <TabsTrigger value="three">{t("Third Tab")}</TabsTrigger>
              </TabsList>
              <TabsContent value="one">
                <p className="text-12 text-muted-foreground pt-2">{t("Content for the first tab.")}</p>
              </TabsContent>
              <TabsContent value="two">
                <p className="text-12 text-muted-foreground pt-2">{t("Content for the second tab.")}</p>
              </TabsContent>
              <TabsContent value="three">
                <p className="text-12 text-muted-foreground pt-2">{t("Content for the third tab.")}</p>
              </TabsContent>
            </Tabs>
          </ComponentShowcase>
        </Section>

        {/* ─── TABLES ─── */}
        <Section id="tables" title={t("Tables")}>
          <ComponentShowcase title={t("Standard Table")}>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("Name")}</TableHead>
                  <TableHead>{t("Status")}</TableHead>
                  <TableHead>{t("Type")}</TableHead>
                  <TableHead className="text-right">{t("Actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow>
                  <TableCell className="font-medium">{t("openai-prod")}</TableCell>
                  <TableCell><Badge variant="success">{t("Active")}</Badge></TableCell>
                  <TableCell>{t("API Key")}</TableCell>
                  <TableCell className="text-right">
                    <Button variant="ghost" size="icon"><Edit className="h-3.5 w-3.5" /></Button>
                    <Button variant="ghost" size="icon"><Trash2 className="h-3.5 w-3.5 text-destructive" /></Button>
                  </TableCell>
                </TableRow>
                <TableRow>
                  <TableCell className="font-medium">{"anthropic-dev"}</TableCell>
                  <TableCell><Badge variant="warning">{t("Expiring")}</Badge></TableCell>
                  <TableCell>{t("OAuth")}</TableCell>
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
        <Section id="dropdowns" title={t("Dropdowns")}>
          <ComponentShowcase title={t("Dropdown Menu")}>
            <div className="flex gap-4">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="outline">{t("Options ")}<ChevronDown className="ml-2 h-3.5 w-3.5" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent className="p-2">
                  <DropdownMenuItem className="rounded-lg"><Edit className="mr-2" />{t(" Edit")}</DropdownMenuItem>
                  <DropdownMenuItem className="rounded-lg"><Copy className="mr-2" />{t(" Duplicate")}</DropdownMenuItem>
                  <DropdownMenuItem className="rounded-lg text-destructive focus:text-destructive [&_svg]:text-destructive"><Trash2 className="mr-2" />{t(" Delete")}</DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            <div className="mt-4 rounded-lg bg-muted/50 p-3">
              <p className="text-11 text-text-tertiary">{t("Container: ")}<code className="text-nyx-secondary-400">p-2 rounded-xl</code>{t(". Items: ")}<code className="text-nyx-secondary-400">rounded-lg px-2 py-2</code>{t(". No separators — use spacing instead.")}</p>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── DIALOGS ─── */}
        <Section id="dialogs" title={t("Dialogs")}>
          <ComponentShowcase title={t("Standard Dialog")}>
            <Dialog>
              <DialogTrigger asChild>
                <Button variant="outline">{t("Open Dialog")}</Button>
              </DialogTrigger>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>{t("Dialog Title")}</DialogTitle>
                  <DialogDescription>{t("A brief description of what this dialog does.")}</DialogDescription>
                </DialogHeader>
                <div className="space-y-4">
                  <div className="space-y-2">
                    <Label>{t("Name")}</Label>
                    <Input placeholder={t("Enter name...")} />
                  </div>
                </div>
                <DialogFooter>
                  <Button variant="ghost">{t("Cancel")}</Button>
                  <Button>{t("Save")}</Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
            <div className="mt-4 rounded-lg bg-muted/50 p-3">
              <p className="text-11 text-text-tertiary">{t("Padding: ")}<code className="text-nyx-secondary-400">p-6</code>{t(". Radius: ")}<code className="text-nyx-secondary-400">rounded-xl</code>{t(". Gap: ")}<code className="text-nyx-secondary-400">gap-6</code>{t(". Overlay: ")}<code className="text-nyx-secondary-400">bg-black/60 backdrop-blur-sm</code>.
              </p>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── TOOLTIPS ─── */}
        <Section id="tooltips" title={t("Tooltips")}>
          <ComponentShowcase title={t("Standard Tooltip")}>
            <div className="flex gap-4">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="outline">{t("Hover me")}</Button>
                </TooltipTrigger>
                <TooltipContent>{t("Tooltip content")}</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon"><Info className="h-4 w-4" /></Button>
                </TooltipTrigger>
                <TooltipContent>{t("Information tooltip")}</TooltipContent>
              </Tooltip>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── SWITCHES ─── */}
        <Section id="switches" title={t("Switches")}>
          <ComponentShowcase title={t("Switch States")}>
            <div className="flex items-center gap-8">
              <div className="flex items-center gap-3">
                <Switch checked={switchVal} onCheckedChange={setSwitchVal} />
                <Label>{t("Enabled")}</Label>
              </div>
              <div className="flex items-center gap-3">
                <Switch checked={false} disabled />
                <Label className="text-text-tertiary">{t("Disabled")}</Label>
              </div>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── SKELETONS ─── */}
        <Section id="skeletons" title={t("Skeletons")}>
          <ComponentShowcase title={t("Loading States")}>
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
        <Section id="icons" title={t("Icon Styling")}>
          <div className="space-y-6">
            <ComponentShowcase title={t("Icon Container Variants")}>
              <div className="flex flex-wrap gap-6 items-center">
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-8 w-8 items-center justify-center rounded-[8px] nyx-gradient-vivid text-white">
                    <Plus className="h-4 w-4" />
                  </div>
                  <span className="text-10 text-text-tertiary">{t("Gradient")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-hairline bg-overlay text-text-tertiary">
                    <Settings className="h-4 w-4" />
                  </div>
                  <span className="text-10 text-text-tertiary">{t("Bordered")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-8 w-8 items-center justify-center rounded-[8px] border border-nyx-500/30 bg-nyx-500/15 text-nyx-secondary-400">
                    <Zap className="h-4 w-4" />
                  </div>
                  <span className="text-10 text-text-tertiary">{t("Brand bordered")}</span>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <div className="flex h-[22px] w-[22px] items-center justify-center rounded-[6px] border border-hairline bg-overlay text-text-tertiary">
                    <Plus className="h-3 w-3" />
                  </div>
                  <span className="text-10 text-text-tertiary">{t("Small (CTA)")}</span>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Icon Sizes")}>
              <div className="flex flex-wrap gap-6 items-end">
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-3 w-3 text-text-tertiary" />
                  <code className="text-10 font-mono text-text-tertiary">12px</code>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-3.5 w-3.5 text-text-tertiary" />
                  <code className="text-10 font-mono text-text-tertiary">14px</code>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-4 w-4 text-text-tertiary" />
                  <code className="text-10 font-mono text-text-tertiary">16px</code>
                </div>
                <div className="flex flex-col items-center gap-2">
                  <Cable className="h-[18px] w-[18px] text-text-tertiary" />
                  <code className="text-10 font-mono text-text-tertiary">18px</code>
                </div>
              </div>
              <div className="mt-4 rounded-lg bg-muted/50 p-3">
                <p className="text-11 text-text-tertiary">{t("Nav sidebar: ")}<code className="text-nyx-secondary-400">16px</code>{t(". Top bar chrome: ")}<code className="text-nyx-secondary-400">18px</code>{t(". Inline buttons: ")}<code className="text-nyx-secondary-400">14px</code>{t(". Stat cards: ")}<code className="text-nyx-secondary-400">14px</code>{t(". Tiny indicators: ")}<code className="text-nyx-secondary-400">12px</code>.
                </p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Platform icon scale")}>
              <div className="space-y-4 overflow-x-auto">
                <div className="min-w-[680px]">
                  <div className="grid grid-cols-[minmax(180px,1fr)_repeat(6,56px)] items-center gap-2 border-b border-border/40 pb-2 text-10 font-mono uppercase tracking-[0.08em] text-text-tertiary">
                    <span>{t("Agent platform")}</span>
                    {ICON_SIZE_OPTIONS.map(({ label }) => (
                      <span key={label} className="text-center">{label}</span>
                    ))}
                  </div>
                  {PLATFORM_ICON_OPTIONS.map(({ platform, label }) => (
                    <div
                      key={platform}
                      className="grid grid-cols-[minmax(180px,1fr)_repeat(6,56px)] items-center gap-2 border-b border-border/20 py-3 last:border-0"
                    >
                      <div className="min-w-0">
                        <p className="truncate text-13 font-medium text-foreground">{label}</p>
                        <code className="text-10 text-text-tertiary">{platform}</code>
                      </div>
                      {ICON_SIZE_OPTIONS.map(({ value, label: sizeLabel }) => (
                        <div key={sizeLabel} className="flex h-10 items-center justify-center">
                          <PlatformIcon platform={platform} size={value} />
                        </div>
                      ))}
                    </div>
                  ))}
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Catalog service icon audit")}>
              <div className="space-y-4">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <p className="text-12 text-muted-foreground">
                    {ICON_AUDIT_SLUGS.length}{t(" registered service marks, including seeded Pro services and internal platform aliases.")}</p>
                  <div className="relative w-full sm:w-64">
                    <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-text-tertiary" />
                    <Input
                      value={iconSearch}
                      onChange={(event) => setIconSearch(event.target.value)}
                      placeholder={t("Filter services")}
                      aria-label={t("Filter service icons")}
                      className="h-8 pl-8 text-12"
                    />
                  </div>
                </div>
                <div className="overflow-x-auto rounded-lg border border-border/50">
                  <div className="min-w-[680px]">
                    <div className="grid grid-cols-[minmax(180px,1fr)_repeat(6,56px)] items-center gap-2 border-b border-border/50 bg-muted/40 px-3 py-2 text-10 font-mono uppercase tracking-[0.08em] text-text-tertiary">
                      <span>{t("Service")}</span>
                      {ICON_SIZE_OPTIONS.map(({ label }) => (
                        <span key={label} className="text-center">{label}</span>
                      ))}
                    </div>
                    {filteredIconAuditSlugs.map((slug) => (
                      <div
                        key={slug}
                        className="grid grid-cols-[minmax(180px,1fr)_repeat(6,56px)] items-center gap-2 border-b border-border/20 px-3 last:border-0"
                      >
                        <div className="min-w-0 py-2">
                          <p className="truncate text-12 font-medium text-foreground">{iconAuditLabel(slug)}</p>
                          <code className="truncate text-10 text-text-tertiary">{slug}</code>
                        </div>
                        {ICON_SIZE_OPTIONS.map(({ value, label }) => (
                          <div key={label} className="flex h-12 items-center justify-center">
                            <ServiceIcon slug={slug} size={value} />
                          </div>
                        ))}
                      </div>
                    ))}
                    {filteredIconAuditSlugs.length === 0 && (
                      <p className="px-3 py-8 text-center text-12 text-muted-foreground">{t("No matching service icons.")}</p>
                    )}
                  </div>
                </div>
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ─── TOASTS ─── */}
        <Section id="toasts" title={t("Toasts")}>
          <ComponentShowcase title={t("Toast Variants")}>
            <div className="flex flex-col gap-3">
              <div className="flex items-center gap-3 rounded-xl border border-success/30 bg-success/[0.06] p-3 text-13 text-success">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-success/20 bg-success/10">
                  <Check className="h-3 w-3" />
                </span>
                <span>{t("Action completed successfully")}</span>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-destructive/30 bg-destructive/[0.06] p-3 text-13 text-destructive">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-destructive/20 bg-destructive/10">
                  <X className="h-3 w-3" />
                </span>
                <span>{t("Something went wrong")}</span>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-nyx-500/30 bg-nyx-500/[0.06] p-3 text-13 text-nyx-secondary-400">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-nyx-500/20 bg-nyx-500/10">
                  <Info className="h-3 w-3" />
                </span>
                <span>{t("Here is some information")}</span>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-warning/30 bg-warning/[0.06] p-3 text-13 text-warning">
                <span className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-warning/20 bg-warning/10">
                  <AlertCircle className="h-3 w-3" />
                </span>
                <span>{t("Please review this")}</span>
              </div>
            </div>
          </ComponentShowcase>
        </Section>

        {/* ─── PATTERNS ─── */}
        <Section id="patterns" title={t("Patterns")}>
          <div className="space-y-6">
            <ComponentShowcase title={t("Section Overline + Content")}>
              <div className="flex flex-col gap-3">
                <p className="text-11 font-semibold uppercase tracking-[1.5px] text-text-tertiary">{t("Section Label")}</p>
                <p className="text-12 text-muted-foreground">{t("Content below the overline label.")}</p>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Status Banner (All Clear)")}>
              <div className="flex items-center gap-3 rounded-xl border border-success/15 bg-success/[0.04] px-4 py-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-success/10">
                  <ShieldCheck className="h-4.5 w-4.5 text-success" />
                </div>
                <div>
                  <p className="text-13 font-semibold text-foreground">{t("All systems clear")}</p>
                  <p className="text-11 text-muted-foreground">{t("Nothing needs your attention right now.")}</p>
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Status Banner (Error)")}>
              <ErrorBanner message={t("Failed to load services")} onRetry={() => {}} />
            </ComponentShowcase>

            <ComponentShowcase title={t("Attention Row")}>
              <div className="flex flex-col gap-2">
                <div className="group flex items-center gap-3 rounded-lg px-3 py-2 transition-colors duration-300 hover:bg-overlay cursor-pointer">
                  <div className="h-1.5 w-1.5 rounded-full shrink-0 bg-warning" />
                  <span className="shrink-0 text-warning"><ShieldAlert className="h-4 w-4" /></span>
                  <span className="text-13 text-foreground flex-1">{t("3 pending approvals")}</span>
                  <ArrowRight className="h-3 w-3 text-text-tertiary opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
                <div className="group flex items-center gap-3 rounded-lg px-3 py-2 transition-colors duration-300 hover:bg-overlay cursor-pointer">
                  <div className="h-1.5 w-1.5 rounded-full shrink-0 bg-error" />
                  <span className="shrink-0 text-error"><AlertCircle className="h-4 w-4" /></span>
                  <span className="text-13 text-foreground flex-1">{t("1 node offline")}</span>
                  <ArrowRight className="h-3 w-3 text-text-tertiary opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Status Chips")}>
              <div className="flex flex-wrap gap-2">
                <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-11 font-medium bg-nyx-500/10 text-nyx-secondary-400 border border-nyx-500/20">
                  <CheckCircle2 className="h-3 w-3 shrink-0" />{t(" Complete")}</div>
                <div className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-11 font-medium text-text-tertiary border border-dashed border-border/40">
                  <div className="h-1.5 w-1.5 shrink-0 rounded-full bg-border/60" />{t(" Incomplete")}</div>
              </div>
            </ComponentShowcase>

            <ComponentShowcase title={t("Empty State")}>
              <div className="flex flex-col items-center gap-4 py-12">
                <div className="flex h-14 w-14 items-center justify-center rounded-xl border border-border">
                  <Cable className="h-6 w-6 text-muted-foreground" />
                </div>
                <div className="text-center max-w-md space-y-1">
                  <p className="text-12 font-medium text-muted-foreground">{t("No items yet")}</p>
                  <p className="text-13 text-muted-foreground">{t("Get started by adding your first item.")}</p>
                </div>
                <AddCtaButton label={t("Add Item")} onClick={() => {}} />
              </div>
            </ComponentShowcase>
          </div>
        </Section>

        {/* ── UX Rules ── */}
        <Section id="ux-rules" title={t("UX Rules")}>
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
                  detail: "All section titles above card grids must use the same pattern: an icon (h-4 w-4, text-muted-foreground) followed by text-13 font-semibold text-foreground. Never mix uppercase tertiary text with this pattern on the same page.",
                  code: '<Icon className="h-4 w-4 text-muted-foreground" />\n<h3 className="text-13 font-semibold text-foreground">Title</h3>',
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
                  code: '"hover:border-hairline-strong hover:bg-accent/30"',
                },
                {
                  rule: "Clickable rows get hover state, non-clickable rows do not",
                  detail: "Table rows that navigate on click use cursor-pointer and a subtle hover background. Non-interactive rows have no hover effect.",
                  code: '"cursor-pointer hover:bg-overlay"',
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
                  code: '"text-11 font-semibold uppercase tracking-[1.5px] text-text-tertiary"',
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
                  code: '"px-4 py-4 align-top text-13 text-foreground"',
                },
                {
                  rule: "Table cell text size is consistent at 13px",
                  detail: "All primary table cell content uses text-13 (set in the base TableCell). Secondary metadata can use text-11 or text-10. Never use text-sm (14px) or text-xs (12px) for primary content.",
                  code: 'Base: text-13 | Secondary: text-11 text-muted-foreground',
                },
                {
                  rule: "Clickable rows have hover state, not hyperlinks",
                  detail: "When a table row navigates on click, the entire row gets cursor-pointer and hover:bg-overlay. The name/title cell should NOT be styled as a hyperlink (no underline, no link color). Non-clickable rows have no hover effect.",
                  code: '<TableRow className="cursor-pointer hover:bg-overlay" onClick={navigate}>',
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
                  <p className="text-13 font-semibold text-foreground">{item.rule}</p>
                  <p className="text-12 text-muted-foreground">{item.detail}</p>
                  <code className="block text-11 font-mono text-nyx-secondary-400 bg-muted rounded-lg px-3 py-2">
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
