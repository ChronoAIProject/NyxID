import { useState } from "react";
import { PageHeader } from "@/components/shared/page-header";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { useAdminBillingConfig, useAdminBillingOverview, useAdminBillingRates, useResyncAdminBillingService, useUpdateAdminBillingConfig } from "@/hooks/use-admin-billing";

type Tab = "overview" | "rates" | "settings";
export function AdminBillingPage() {
  const [tab, setTab] = useState<Tab>("overview");
  const [search, setSearch] = useState("");
  const [cap, setCap] = useState("");
  const overview = useAdminBillingOverview(); const rates = useAdminBillingRates(); const config = useAdminBillingConfig(); const update = useUpdateAdminBillingConfig(); const resync = useResyncAdminBillingService();
  return <div className="space-y-6"><PageHeader title="Billing" description="Central configuration with explicit precedence." /><div role="tablist" aria-label="Billing sections" className="flex gap-2">{(["overview", "rates", "settings"] as const).map((value) => <Button key={value} aria-selected={tab === value} variant={tab === value ? "default" : "outline"} onClick={() => setTab(value)}>{value.charAt(0).toUpperCase() + value.slice(1)}</Button>)}</div>
    {tab === "overview" && <Card><CardHeader><CardTitle>Services</CardTitle><Input placeholder="Search services" value={search} onChange={(event) => setSearch(event.target.value)} /></CardHeader><CardContent className="space-y-2">{overview.isLoading && <p>Loading services…</p>}{overview.data?.filter((service) => `${service.name} ${service.slug}`.toLowerCase().includes(search.toLowerCase())).map((service) => <div className="flex items-center justify-between border-b py-2" key={service.id}><span>{service.name} <small>({service.slug})</small></span><span className="text-muted-foreground">source: {service.effective_source}</span><Button size="sm" variant="outline" onClick={() => resync.mutate(service.id)} disabled={resync.isPending}>Resync</Button></div>)}{!overview.isLoading && !overview.data?.length && <p>No services have billing configuration.</p>}</CardContent></Card>}
    {tab === "rates" && <Card><CardHeader><CardTitle>Rate cache</CardTitle></CardHeader><CardContent>{rates.data?.map((rate) => <div className="flex justify-between border-b py-2" key={rate.id}><span>{rate.metric}{rate.model ? ` / ${rate.model}` : ""}</span><span>{rate.stale ? "stale" : "current"}{rate.retired_at ? " · retired" : ""}</span></div>)}</CardContent></Card>}
    {tab === "settings" && <Card><CardHeader><CardTitle>Settings</CardTitle></CardHeader><CardContent className="space-y-3"><p>Environment values are read-only. Global defaults apply when a service has no override.</p><p>Billing enabled: {String(config.data?.billing_enabled)} · Lago configured: {String(config.data?.lago_configured)}</p><label htmlFor="overdraft-cap">Global overdraft cap</label><Input id="overdraft-cap" inputMode="numeric" value={cap} onChange={(event) => setCap(event.target.value)} /><Button disabled={update.isPending} onClick={() => update.mutate({ default_lane: config.data?.global?.default_lane ?? null, default_overdraft_cap_credits: cap === "" ? null : Number(cap) })}>Save global defaults</Button></CardContent></Card>}
  </div>;
}
