vi.mock("./composer-editor", () => import("./composer-editor.test-support"));
vi.mock("@/hooks/use-keys", () => ({ useKeys: () => ({data: [], isError: false}) }));
// Exercise the composer's submit boundary independently of settings persistence.
// The real menu and responsive geometry are covered by composer-settings e2e.
vi.mock("./composer-settings", () => ({ ComposerSettings: ({connectionChoice}: {
  connectionChoice: {value:string;onChange:(id:string)=>void};
}) => <button type="button" onClick={() => connectionChoice.onChange("org-connection")}
  aria-label="Choose exact organization connection">{connectionChoice.value || "Saved order"}</button> }));
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NyxAgentComposer } from "./nyxagent-composer";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";

beforeEach(() => useAssistantDraftStore.setState({ownerUserId:null,drafts:{}}));
function mount(onSend=vi.fn().mockResolvedValue(undefined)) {
  const query=new QueryClient({defaultOptions:{queries:{retry:false}}});
  const props={scope:{kind:"conversations" as const,id:"selection-test"},ownerUserId:"owner",draftKey:"selection:test",active:false,sending:false,onSend,
    onStop:vi.fn().mockResolvedValue(undefined)};
  const view=render(<QueryClientProvider client={query}><NyxAgentComposer {...props}/></QueryClientProvider>);
  return {onSend,rerender:(draftKey:string) => view.rerender(<QueryClientProvider client={query}><NyxAgentComposer {...props} draftKey={draftKey}/></QueryClientProvider>)};
}
async function submit(text:string) {
  fireEvent.change(screen.getByRole("textbox"),{target:{value:text}});
  fireEvent.click(screen.getByRole("button",{name:"Send message"}));
}
describe("one-message exact connection selection",() => {
  it("submits an actual newline selector and clears selection only after success",async () => {
    const view=mount();fireEvent.click(screen.getByRole("button",{name:"Choose exact organization connection"}));
    await submit("Use this account");
    await waitFor(() => expect(view.onSend).toHaveBeenCalledWith("/service org-connection\nUse this account",undefined,undefined));
    await waitFor(() => expect(screen.getByRole("button",{name:"Choose exact organization connection"})).toHaveTextContent("Saved order"));
    await submit("Another operation");
    await waitFor(() => expect(view.onSend).toHaveBeenLastCalledWith("Another operation",undefined,undefined));
  });
  it("retains the exact selection after a failed send",async () => {
    const send=vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(undefined);
    const view=mount(send);fireEvent.click(screen.getByRole("button",{name:"Choose exact organization connection"}));
    await submit("Keep my account");await waitFor(() => expect(send).toHaveBeenCalledOnce());
    expect(screen.getByRole("button",{name:"Choose exact organization connection"})).toHaveTextContent("org-connection");
    await waitFor(() => expect(screen.getByRole("button",{name:"Send message"})).toBeEnabled());
    fireEvent.click(screen.getByRole("button",{name:"Send message"}));
    await waitFor(() => expect(view.onSend).toHaveBeenCalledTimes(2));
    expect(send.mock.calls[0]?.[0]).toBe(send.mock.calls[1]?.[0]);
  });
  it("resets selection when navigating to another draft",async () => {
    const view=mount();fireEvent.click(screen.getByRole("button",{name:"Choose exact organization connection"}));
    view.rerender("selection:other");
    await waitFor(() => expect(screen.getByRole("button",{name:"Choose exact organization connection"})).toHaveTextContent("Saved order"));
  });
});
