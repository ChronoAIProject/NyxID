import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { nyxAgentChannelAgentSchema } from "@/schemas/assistant-nyxagent";
import { ChannelChats } from "./nyxbot-channel-chats";

let client: QueryClient;
let reads: string[];
let scoped: string[];
const base = "/assistant/nyxagent/agents/agent/guest-readiness";
const at = "2026-10-29T00:00:00Z";
beforeEach(() => {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useAuthStore.getState().setUser({ id:"owner",email:"owner@example.com",display_name:"Owner",avatar_url:null,
    email_verified:true,mfa_enabled:false,is_admin:false,is_active:true,created_at:at });
  reads=[]; scoped=[];
  globalThis.__nyxidAssistantHttpMock = ({ endpoint }) => {
    reads.push(endpoint);
    let value: unknown;
    if (endpoint === base) value = { status:"ready",conversation_overrides:scoped };
    else if (endpoint === `${base}?conversation_id=conv-1`) value = { status:"not_ready",fix:"Confirm this chat's model route." };
    else if (endpoint.endsWith("/chats")) value = { chats:Array.from({length:20},(_,i)=>({
      id:`chat-${i}`,channel_agent_id:"channel",platform:"telegram-new",bot_label:"Visitor bot",kind:"private",
      title:`Visitor ${i}`,members:"everyone",conversation_id:`conv-${i}`,
    })) };
    else return new Response("{}",{status:404});
    return new Response(JSON.stringify(value));
  };
});
afterEach(() => { cleanup();client.clear();globalThis.__nyxidAssistantHttpMock=undefined;useAuthStore.getState().setUser(null); });
function mount() {
  const channel=nyxAgentChannelAgentSchema.parse({ id:"channel",channel_bot_id:"bot",agent_id:"agent",platform:"telegram-new",
    bot_label:"Visitor bot",transport:"gateway",status:"active",owner_linked:true,created_at:at,private_chats:"everyone",guest_readiness:null });
  render(<QueryClientProvider client={client}><ChannelChats channel={channel} agents={[]} botAgentName="Helper" /></QueryClientProvider>);
  return userEvent.setup();
}
it("loads channel readiness only when opened and shares it across ordinary chat rows", async () => {
  const user=mount();
  expect(reads).toEqual([]);
  await user.click(screen.getByRole("button",{name:"Chats and who can talk"}));
  await screen.findByText("Visitor 7");
  expect(screen.getAllByText("Model ready for visitors")).toHaveLength(9);
  expect(reads.filter((r)=>r.includes("guest-readiness"))).toEqual([base]);
  await user.click(screen.getByRole("button",{name:"Show 12 more"}));
  await screen.findByText("Visitor 19");
  expect(reads.filter((r)=>r.includes("guest-readiness"))).toEqual([base]);
});
it("checks a chat separately only when the server reports its own profile or route", async () => {
  scoped=["conv-1"];
  const user=mount();
  await user.click(screen.getByRole("button",{name:"Chats and who can talk"}));
  expect(await screen.findByText("Confirm this chat's model route.")).toBeVisible();
  await waitFor(()=>expect(reads.filter((r)=>r.includes("guest-readiness"))).toEqual([base,`${base}?conversation_id=conv-1`]));
});
