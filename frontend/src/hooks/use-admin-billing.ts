import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { billingConfigSchema, billingOverviewSchema, billingRateSchema, billingSettingsSchema, type BillingSettings } from "@/schemas/admin-billing";
const key=["admin","billing"] as const;
export const useAdminBillingOverview=()=>useQuery({queryKey:[...key,"overview"],queryFn:async()=>billingOverviewSchema.parse(await api.get("/admin/billing/overview"))});
export const useAdminBillingRates=()=>useQuery({queryKey:[...key,"rates"],queryFn:async()=>billingRateSchema.array().parse(await api.get("/admin/billing/rates"))});
export const useAdminBillingConfig=()=>useQuery({queryKey:[...key,"config"],queryFn:async()=>billingConfigSchema.parse(await api.get("/admin/billing/config"))});
export const useUpdateAdminBillingConfig=()=>{const qc=useQueryClient();return useMutation({mutationFn:async(billing:BillingSettings)=>billingConfigSchema.parse(await api.put("/admin/billing/config",{billing:billingSettingsSchema.parse(billing)})),onSuccess:()=>qc.invalidateQueries({queryKey:key})});};
export const useResyncAdminBillingService=()=>{const qc=useQueryClient();return useMutation({mutationFn:(id:string)=>api.post(`/admin/billing/services/${encodeURIComponent(id)}/resync`,{}),onSuccess:()=>qc.invalidateQueries({queryKey:key})});};
