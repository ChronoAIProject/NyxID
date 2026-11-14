import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ConsentBanner } from "./consent-banner";
import { useConsentStore } from "@/stores/consent-store";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
const mocks = vi.hoisted(() => ({read:vi.fn(),save:vi.fn()}));
vi.mock("@/lib/analytics-consent",()=>({readAnalyticsConsent:mocks.read,saveAnalyticsConsent:mocks.save}));
vi.mock("@/hooks/use-public-config",()=>({usePublicConfig:()=>({data:{telemetry_share_analytics:true}})}));
const original = "Optional usage analytics are linked to your account. Allow enables account permission and this browser. Declining withdraws account permission across devices. Each other device still needs its own opt-in, and Do Not Track or Global Privacy Control always disables collection. Change this in Settings → Privacy.";
const initialAuth=useAuthStore.getState(),initialConsent=useConsentStore.getState();
let client:QueryClient;
beforeEach(()=>{
  localStorage.clear();
  useAuthStore.setState({user:{id:"owner"} as User});
  useConsentStore.setState({accountId:"owner",asked:false,enabled:false,policy:1});
  mocks.read.mockReset().mockResolvedValue({account_id:"owner",enabled:true,policy_version:"1",current_policy_version:"1",diagnostics_offered:false});
  mocks.save.mockReset().mockResolvedValue({});
  client=new QueryClient({defaultOptions:{queries:{retry:false}}});
});
afterEach(()=>{cleanup();client.clear();useAuthStore.setState(initialAuth);useConsentStore.setState(initialConsent);});
function show(){return render(<QueryClientProvider client={client}><ConsentBanner/></QueryClientProvider>);}
it("flag-off first-time banner retains the exact existing disclosure and account/browser choice",async()=>{
  show();
  await waitFor(()=>expect(mocks.read).toHaveBeenCalledOnce());
  expect(screen.getByText(original)).toBeInTheDocument();
  expect(screen.queryByText(/Analytics updated/)).not.toBeInTheDocument();
  expect(screen.queryByText(/crash diagnostics/i)).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button",{name:"Decline telemetry"}));
  expect(mocks.save).toHaveBeenCalledWith("owner",false,1);
});
it("declined users cause no consent GET from the banner",async()=>{
  useConsentStore.setState({asked:true,enabled:false});
  const view=show();
  await act(async()=>{window.dispatchEvent(new Event("focus"));await Promise.resolve();});
  view.rerender(<QueryClientProvider client={client}><ConsentBanner/></QueryClientProvider>);
  expect(mocks.read).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
it("first-time offered policy-2 disclosure is complete and does not claim an update",async()=>{
  mocks.read.mockResolvedValue({account_id:"owner",enabled:false,current_policy_version:"2",diagnostics_offered:true});
  show();
  await screen.findByText(/Optional crash diagnostics include app version/);
  const text=screen.getByRole("dialog").textContent!;
  for(const part of ["linked to your account","account permission and this browser","Declining withdraws account permission across devices","Each other device still needs its own opt-in","Do Not Track or Global Privacy Control","sanitized code locations","up to 30 days"])
    expect(text).toContain(part);
  expect(text).not.toContain("Analytics updated");
  expect(screen.getByRole("button",{name:"Accept telemetry"})).toHaveTextContent("Allow");
  await userEvent.click(screen.getByRole("button",{name:"Accept telemetry"}));
  expect(mocks.save).toHaveBeenCalledWith("owner",true,2);
});
it("flag off hides re-acknowledgement while an offered existing choice gets the updated variant",async()=>{
  useConsentStore.setState({asked:true,enabled:true});
  show();
  await waitFor(()=>expect(mocks.read).toHaveBeenCalledOnce());
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await act(async()=>{client.setQueryData(["analytics-consent","owner"],{account_id:"owner",enabled:true,policy_version:"1",current_policy_version:"2",diagnostics_offered:true});});
  expect(screen.getByText(/^Analytics updated:/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button",{name:"Decline telemetry"}));
  expect(mocks.save).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
it("a fresh banner cache is reused and window focus does not refetch",async()=>{
  const first=show();
  await waitFor(()=>expect(mocks.read).toHaveBeenCalledOnce());
  first.unmount();show();
  await act(async()=>{window.dispatchEvent(new Event("focus"));await Promise.resolve();});
  expect(mocks.read).toHaveBeenCalledOnce();
});
