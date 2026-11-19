import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { MutationObserver, QueryClient, QueryObserver } from "@tanstack/query-core";
import { ApiError } from "../../lib/api/ApiError";
import { translate } from "../../i18n/translate";
import type { Language } from "../../i18n/language";
import { lightColors } from "../../theme/mobileTheme";

type Element = { type: unknown; props: Record<string, any> };
const elements = (node: Element): Element[] => [
  node,
  ...[node.props.children]
    .flat(Infinity)
    .flatMap((child) => (child?.props ? elements(child) : [])),
];
/** Render the screen's own small row components (no hooks of their own beyond mocks). */
const expand = (node: Element): Element => {
  if (typeof node.type === "function" && ["AddRow", "OrderButton"].includes(node.type.name))
    return expand((node.type as (props: object) => Element)(node.props));
  const render = (child: unknown): unknown =>
    Array.isArray(child)
      ? child.map(render)
      : child && typeof child === "object" && "props" in child
        ? expand(child as Element)
        : child;
  return { ...node, props: { ...node.props, children: render(node.props.children) } };
};
// The harness evaluates the screen in its own realm: compare objects as JSON.
const same = (actual: unknown, expected: unknown, message = "") =>
  assert.equal(JSON.stringify(actual), JSON.stringify(expected), message);

const AGENT = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const OTHER = "22222222-3333-4444-8555-666666666666";
const GONE = "11111111-2222-4333-8444-555555555555";
const CREW = "nyxg-0123456789abcdef0123456789abcdef";
const destination = (id: string) => ({ kind: "destination", id });
const DEFAULT = ["new_chat", "notifications", "scan_qr", "recent_chats", "settings"].map(destination);

function setup({
  language = 'en' as Language,
  items = DEFAULT,
  customized = false,
  loaded = true,
  hasUnsupportedEntries = false,
  loadError = null as unknown,
  saveError = null as unknown,
  conflictRecovered = false,
  sources = {
    agents: [
      { id: AGENT, name: "researcher", display_name: "Research", kind: "specialist", status: "idle" },
      { id: OTHER, name: "nyxbot", display_name: "NyxBot", kind: "nyxbot", status: "running" },
    ],
    crews: [{ id: CREW, name: "Launch crew", members: [] }],
  } as Record<string, unknown>,
} = {}) {
  const calls: unknown[][] = [];
  const quick = {
    state: { items, customized, hasUnsupportedEntries },
    loaded,
    loadError,
    saveError,
    conflictRecovered,
    save: (next: unknown, complete?: () => void) => { calls.push(["save", next]); complete?.(); },
    reset: (complete?: () => void) => { calls.push(["reset"]); complete?.(); },
    clearSaveError: () => calls.push(["clear"]),
    retry: () => calls.push(["retry"]),
  };
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const h = hookHarness<Element>({
    "../../i18n/LanguageProvider": { useAppLanguage: () => ({language, t: (phrase: string, values?: Record<string, string | number>) => translate(language, phrase, values)}) },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Pressable: "Pressable",
      View: "View",
      StyleSheet: { create: (value: unknown) => value },
    },
    "lucide-react-native": Object.fromEntries(
      ["ChevronDown", "ChevronUp", "Minus", "Plus"].map((name) => [name, name]),
    ),
    "../../components/settings/SettingsList": Object.fromEntries(
      ["SettingsPage", "SettingsGroup", "SettingsRow"].map((name) => [name, name]),
    ),
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "./QuickAccessGlyph": { QuickAccessGlyph: "QuickAccessGlyph" },
    "./useQuickAccess": { useQuickAccess: () => quick },
    "./useQuickAccessSources": {
      useQuickAccessSources: () => sources,
    },
  });
  const { QuickAccessSettingsScreen } = h.load(
    fileURLToPath(new URL("./QuickAccessSettingsScreen.tsx", import.meta.url)),
  ) as { QuickAccessSettingsScreen: (props: unknown) => Element };
  h.mount(() => QuickAccessSettingsScreen({}));
  const page = expand(h.value());
  const groups = elements(page).filter((n) => n.type === "SettingsGroup");
  const group = (title: string | undefined) =>
    groups.find((g) => g.props.title === title)!;
  const rows = (g: Element) => elements(g).filter((n) => n.type === "SettingsRow");
  return { page, groups, group, rows, calls };
}

const orderButtons = (row: Element) =>
  elements(expand(row.props.accessory)).filter((n) => n.type === "Pressable");

test("the rail's list in order: each item moves up, down or out, and every change saves the new order", () => {
  const x = setup();
  const current = x.rows(x.group("Your shortcuts"));
  assert.deepEqual(
    current.map((row) => row.props.label),
    ["New chat", "Notifications", "Scan QR code", "Recent chats", "Settings"],
  );
  // Each shows the rail's mark (the side menu's icon).
  assert.equal(current[0]!.props.leading.type, "QuickAccessGlyph");
  const [up, down, remove] = orderButtons(current[0]!);
  // Complete spoken labels; the first cannot move up, the last not down.
  same(
    [up, down, remove].map((button) => [button!.props.accessibilityLabel, button!.props.disabled]),
    [
      ["Move New chat up", true],
      ["Move New chat down", false],
      ["Remove New chat", false],
    ],
  );
  assert.equal(orderButtons(current.at(-1)!)[1]!.props.disabled, true);
  down!.props.onPress();
  orderButtons(current[2]!)[2]!.props.onPress();
  same(x.calls, [
    ["clear"],
    ["save", [destination("notifications"), destination("new_chat"), ...DEFAULT.slice(2)]],
    ["clear"],
    ["save", [...DEFAULT.slice(0, 2), ...DEFAULT.slice(3)]],
  ]);
  // The footer says where it shows and that it syncs.
  assert.match(x.group("Your shortcuts").props.footer, /Saved to your account/);
});

test("add from the menu, an agent or a crew; only what is not there yet, live agents only; at most 12", () => {
  const x = setup({ items: [destination("new_chat"), { kind: "agent", id: AGENT }] });
  const menu = x.rows(x.group("Add from the menu"));
  assert.equal(menu.length, 12, "13 destinations, minus New chat");
  assert.equal(menu.some((row) => row.props.label === "New chat"), false);
  const machines = menu.find((row) => row.props.label === "Machines")!;
  assert.equal(machines.props.accessibilityLabel, "Add Machines");
  assert.equal(machines.props.disabled, false);
  machines.props.onPress();
  assert.deepEqual(
    x.rows(x.group("Add an agent")).map((row) => row.props.label),
    ["NyxBot"],
    "Research is already pinned",
  );
  x.rows(x.group("Add an agent"))[0]!.props.onPress();
  x.rows(x.group("Add a crew"))[0]!.props.onPress();
  same(x.calls.filter(([kind]) => kind === "save"), [
    ["save", [destination("new_chat"), { kind: "agent", id: AGENT }, destination("machines")]],
    ["save", [destination("new_chat"), { kind: "agent", id: AGENT }, { kind: "agent", id: OTHER }]],
    ["save", [destination("new_chat"), { kind: "agent", id: AGENT }, { kind: "crew", id: CREW }]],
  ]);
  // Full: every add is disabled and the footer says why.
  const twelve = [
    "new_chat", "notifications", "scan_qr", "terminal_login_code", "recent_chats", "automations",
    "machines", "plugins", "activity", "create_agent", "settings", "agents",
  ].map(destination);
  const full = setup({ items: twelve, customized: true });
  assert.ok(full.rows(full.group("Add from the menu")).every((row) => row.props.disabled));
  assert.ok(full.rows(full.group("Add an agent")).every((row) => row.props.disabled));
  assert.match(full.group("Add from the menu").props.footer, /up to 12 items/);
});

test("a deleted agent or crew stays listed (so it can be removed) but is marked as not shown", () => {
  const x = setup({
    items: [{ kind: "agent", id: GONE }, { kind: "crew", id: "nyxg-ffffffffffffffffffffffffffffffff" }],
    customized: true,
  });
  const [agent, crew] = x.rows(x.group("Your shortcuts"));
  assert.equal(agent!.props.label, "Agent unavailable");
  assert.equal(crew!.props.label, "Crew unavailable");
  assert.match(agent!.props.detail, /deleted or you no longer have access/);
  assert.equal(orderButtons(agent!)[2]!.props.accessibilityLabel, "Remove Agent unavailable");
  // While the lists load, they read as loading rather than unavailable.
  const loading = setup({
    items: [{ kind: "agent", id: AGENT }],
    customized: true,
    sources: {},
  });
  const row = loading.rows(loading.group("Your shortcuts"))[0]!;
  assert.equal(row.props.label, "Loading…");
  assert.equal(row.props.detail, undefined);
});

test("reset returns to the default; editing waits for the saved list; failures are explained", () => {
  const fresh = setup();
  const reset = (x: ReturnType<typeof setup>) =>
    x.groups
      .flatMap((g) => x.rows(g))
      .find((row) => row.props.label === "Reset to default")!;
  assert.equal(reset(fresh).props.disabled, true, "already the default");
  const custom = setup({ items: [destination("plugins")], customized: true });
  assert.equal(reset(custom).props.disabled, false);
  reset(custom).props.onPress();
  same(custom.calls, [["clear"], ["reset"]]);
  // Before the saved list loads nothing can be changed over the placeholder.
  const loading = setup({ loaded: false });
  assert.deepEqual(
    loading.rows(loading.group("Your shortcuts")).map((row) => row.props.label),
    ["Loading…"],
  );
  assert.ok(loading.rows(loading.group("Add from the menu")).every((row) => row.props.disabled));
  assert.equal(reset(loading).props.disabled, true);
  // A failed load offers Retry; a failed save says the old list is back.
  const failed = setup({ loaded: false, loadError: new Error("offline") });
  const retry = failed.groups.flatMap((g) => failed.rows(g)).find((row) => row.props.label === "Retry")!;
  retry.props.onPress();
  same(failed.calls, [["retry"]]);
  const unsaved = setup({ saveError: new Error("offline"), customized: true });
  assert.equal(
    unsaved.group("Your shortcuts").props.footer,
    "Couldn't save quick access. Your previous list is back.",
  );
  // An empty list is shown as such.
  const empty = setup({ items: [], customized: true });
  assert.match(empty.rows(empty.group("Your shortcuts"))[0]!.props.label, /Nothing in quick access yet/);
});


test('M2622-05 three accepted UI edits survive staggered server responses; pending controls refuse extra presses', async () => {
  const client=new QueryClient(); const key=['account','quick-access',7];
  client.setQueryData(key,{customized:true,items:[]});
  let server:any={customized:true,items:[]}; const requests:any[]=[];const gates:Array<()=>void>=[];
  let observer:any; const hooks:any={};
  const jsx=(type:unknown,props:any)=>({type,props});
  const h=hookHarness<Element>({
    'react/jsx-runtime':{jsx,jsxs:jsx},
    'react-native':{AppState:{currentState:'active',addEventListener:()=>({remove(){}})},Pressable:'Pressable',View:'View',StyleSheet:{create:(v:unknown)=>v}},
    'lucide-react-native':Object.fromEntries(['ChevronDown','ChevronUp','Minus','Plus'].map(k=>[k,k])),
    '../../components/settings/SettingsList':Object.fromEntries(['SettingsPage','SettingsGroup','SettingsRow'].map(k=>[k,k])),
    '../../theme/ThemeContext':{useTheme:()=>({colors:lightColors})},
    './QuickAccessGlyph':{QuickAccessGlyph:'QuickAccessGlyph'},
    './useQuickAccess':hooks,'./useQuickAccessSources':{useQuickAccessSources:()=>({agents:[],crews:[]})},
    '../../lib/auth/authEpoch':{getAuthEpoch:()=>7},'../../lib/auth/authAccount':{getAuthAccountId:()=> 'a'},
    '../../lib/auth/sessionStore':{authSessionGeneration:()=>1},
    '../../lib/api/quickAccessApi':{quickAccessApi:{save:async(items:any)=>{
      requests.push(items);await new Promise<void>(r=>gates.push(r));server={customized:true,items};return server;
    },reset:async()=>{requests.push('reset');return server;}}},
    '@tanstack/react-query':{useIsMutating:(filters:any)=>client.isMutating(filters),useQueryClient:()=>client,useQuery:()=>({data:client.getQueryData(key)}),useMutation:(opts:any)=>{
      if(!observer){observer=new MutationObserver(client,opts);observer.subscribe(()=>{});}
      return {...observer.getCurrentResult(),mutate:(v:unknown, callbacks:any)=>{void observer.mutate(v,callbacks).catch(()=>{});},reset:()=>observer.reset()};
    }},
  });
  Object.assign(hooks,h.load(fileURLToPath(new URL('./useQuickAccess.ts',import.meta.url))));
  const {QuickAccessSettingsScreen}=h.load(fileURLToPath(new URL('./QuickAccessSettingsScreen.tsx',import.meta.url))) as any;
  h.mount(()=>QuickAccessSettingsScreen({}));
  const tree=()=>elements(expand(h.value())).flatMap(n=>[n,...(n.props.accessory?.props ? elements(expand(n.props.accessory)) : [])]);
  const button=(label:string)=>{const found=tree().find(n=>n.props.accessibilityLabel===label || n.props.label===label);assert.ok(found,label);return found;};
  const settle=async()=>{await new Promise(r=>setImmediate(r));h.render();await h.settle();};
  try {
    const first=button('Add Plugins'),stale=button('Add Machines');first.props.onPress();stale.props.onPress();
    await settle();assert.equal(requests.length,1,'sync guard refuses same-frame second press');
    assert.equal(button('Add Machines').props.disabled,true);assert.equal(button('Reset to default').props.disabled,true);
    assert.equal(button('Remove Plugins').props.disabled,true);
    gates.shift()!();await settle();
    button('Add Machines').props.onPress();await settle();
    assert.equal(button('Add Settings').props.disabled,true);button('Add Settings').props.onPress();await settle();
    assert.equal(requests.length,2,'pending Settings press refused');
    gates.shift()!();await settle();button('Add Settings').props.onPress();await settle();
    gates.shift()!();await settle();
    same(server.items.map((i:any)=>i.id),['plugins','machines','settings']);
    same((client.getQueryData(key) as any).items,server.items,'server readback retains all three accepted edits');
  } finally {h.unmount();client.clear();}
});


test('M2622-08 partial quick-access list stays visible and read-only with explicit Reset available',()=>{
  const x=setup({items:[destination('plugins')],customized:true,hasUnsupportedEntries:true});
  const rows=x.rows(x.group('Your shortcuts'));
  assert.equal(rows[0]!.props.label,'Plugins');
  assert.match(x.group('Your shortcuts').props.footer,/newer version.*Reset/);
  for(const button of orderButtons(rows[0]!)){assert.equal(button.props.disabled,true);button.props.onPress();}
  for(const group of ['Add from the menu','Add an agent','Add a crew']) for(const row of x.rows(x.group(group))){assert.equal(row.props.disabled,true);row.props.onPress();}
  assert.equal(x.calls.length,0,'even forced edit callbacks cannot send partial replacement');
  const reset=x.groups.flatMap(g=>x.rows(g)).find(row=>row.props.label==='Reset to default')!;
  assert.equal(reset.props.disabled,false);reset.props.onPress();
  same(x.calls,[['clear'],['reset']]);
});


test('R1-F4 shortcut conflicts use localized copy while partial lists keep their explanation', () => {
  const message = 'Your shortcuts changed on another device. The latest list is shown. Try again.';
  const error = new ApiError({statusCode: 409, errorCode: 1004, errorKey: 'conflict', message: 'Quick access changed. Reload the list before editing.'});
  for (const language of ['en', 'zh-CN', 'zh-TW'] as const) {
    const x = setup({language, saveError: error, conflictRecovered: true});
    const footer = x.group(translate(language, 'Your shortcuts')).props.footer;
    assert.equal(footer, translate(language, message));
    if (language !== 'en') assert.notEqual(footer, message);
    const partial = setup({language, saveError: error, hasUnsupportedEntries: true});
    assert.equal(partial.group(translate(language, 'Your shortcuts')).props.footer,
      translate(language, 'Some shortcuts need a newer version of NyxID. Update the app to edit them, or use Reset to default to replace the entire list.'));
  }
});


for (const recovery of ['pending', 'failed'] as const) {
  test(`R1-D conflict recovery survives ${recovery} GET and editor remount before a fresh edit`, async () => {
    const client = new QueryClient();
    const key = ['account', 'quick-access', 7];
    const state = (...ids: string[]) => ({items: ids.map(destination), customized: true});
    let server = state('plugins');
    client.setQueryData(key, server);
    let answer!: (value: unknown) => void;
    let reject!: (error: unknown) => void;
    let response = new Promise((yes, no) => {answer = yes; reject = no;});
    let reads = 0;
    const writes: string[][] = [];
    let conflictOnNext = false;
    const api = {
      get: async () => {reads++; return response;},
      save: async (items: any[], guard: () => void) => {
        guard(); writes.push(items.map(item => item.id));
        if (writes.length === 1 || conflictOnNext) {
          conflictOnNext = false;
          server = state('plugins', 'settings');
          throw new ApiError({statusCode:409,errorCode:1004,errorKey:'conflict',message:'Quick access changed.'});
        }
        server = {items, customized:true}; return server;
      },
      reset: async () => {server = state('new_chat'); return server;},
    };
    const mount = () => {
      const queries = new Map<string, QueryObserver<any>>(); let mutation: MutationObserver<any, any, any>;
      const hooks: any = {};
      const jsx = (type: unknown, props: any) => ({type, props});
      const h = hookHarness<Element>({
        'react/jsx-runtime': {jsx,jsxs:jsx},
        'react-native': {Pressable:'Pressable',View:'View',StyleSheet:{create:(v:unknown)=>v}, AppState:{currentState:'active',addEventListener:()=>({remove(){}})}},
        'lucide-react-native':Object.fromEntries(['ChevronDown','ChevronUp','Minus','Plus'].map(k=>[k,k])),
        '../../components/settings/SettingsList':Object.fromEntries(['SettingsPage','SettingsGroup','SettingsRow'].map(k=>[k,k])),
        '../../theme/ThemeContext':{useTheme:()=>({colors:lightColors})},
        './QuickAccessGlyph':{QuickAccessGlyph:'QuickAccessGlyph'}, './useQuickAccess':hooks,
        './useQuickAccessSources':{useQuickAccessSources:()=>({agents:[],crews:[]})},
        '../../lib/auth/authEpoch':{getAuthEpoch:()=>7}, '../../lib/auth/authAccount':{getAuthAccountId:()=>'a'},
        '../../lib/auth/sessionStore':{authSessionGeneration:()=>1}, '../../lib/api/quickAccessApi':{quickAccessApi:api},
        '@tanstack/react-query':{
          useQueryClient:()=>client, useIsMutating:(filters:any)=>client.isMutating(filters),
          useQuery:(options:any)=>{
            const id=JSON.stringify(options.queryKey);
            if(!queries.has(id)) {const q=new QueryObserver(client,{...options,retry:false,retryOnMount:false});queries.set(id,q);q.subscribe(()=>{});}
            return queries.get(id)!.getCurrentResult();
          },
          useMutation:(options:any)=>{
            if(!mutation) {mutation=new MutationObserver(client,options);mutation.subscribe(()=>{});}
            return {...mutation.getCurrentResult(),mutate:(v:unknown,callbacks:any)=>{void mutation.mutate(v,callbacks).catch(()=>{});},reset:()=>mutation.reset()};
          },
        },
      });
      Object.assign(hooks,h.load(fileURLToPath(new URL('./useQuickAccess.ts',import.meta.url))));
      const {QuickAccessSettingsScreen}=h.load(fileURLToPath(new URL('./QuickAccessSettingsScreen.tsx',import.meta.url))) as any;
      h.mount(()=>QuickAccessSettingsScreen({}));
      return {h,hooks,
        all:()=>elements(expand(h.value())),
        close:()=>{h.unmount();queries.forEach(q=>q.destroy());mutation?.reset();},
      };
    };
    let screen = mount();
    const render = async () => {await new Promise(resolve=>setImmediate(resolve));screen.h.render();await screen.h.settle();};
    const add = () => screen.all().find(n=>n.props.accessibilityLabel==='Add Machines')!;
    const footer = () => screen.all().find(n=>n.props.title==='Your shortcuts')!.props.footer;
    try {
      const staleEdit = add().props.onPress;
      staleEdit(); await render();
      assert.ok(reads>0,'conflict starts an authoritative GET');
      if(recovery==='failed') {reject(Error('offline'));await render();}
      screen.close(); screen=mount(); await render();
      assert.equal(add().props.disabled,true,'ordinary edits remain fenced after remount');
      assert.doesNotMatch(footer(),/latest list is shown/i,'copy cannot claim a fresh list before GET success');
      add().props.onPress(); staleEdit(); await render();assert.equal(writes.length,1,'even forced stale callbacks cannot write');
      const reset=screen.all().find(n=>n.props.label==='Reset to default')!;
      assert.equal(reset.props.disabled,false,'explicit whole-list Reset remains available');
      if(recovery==='failed') {
        assert.match(footer(),/reload|load/i);
        const retry=screen.all().find(n=>n.props.label==='Reload shortcuts')!;assert.ok(retry,'failed recovery offers GET retry');
        response=Promise.resolve(server);const before=reads;retry.props.onPress();await render();assert.ok(reads>before,'retry reloads instead of writing');
      } else {answer(server);await render();}
      assert.equal(add().props.disabled,false);
      assert.match(footer(),/latest list is shown/i);
      same((client.getQueryData(key) as any).items.map((i:any)=>i.id),['plugins','settings']);
      add().props.onPress();await render();
      same(server.items.map(i=>i.id),['plugins','settings','machines'],'fresh user edit keeps competing Settings');
      assert.equal(writes.length,2);
      // Reset is an explicit replacement, and cancels a delayed conflict read.
      conflictOnNext = true;
      response = new Promise(yes => {answer = yes;});
      screen.all().find(n=>n.props.accessibilityLabel==='Add Notifications')!.props.onPress();await render();
      const competing = server;
      screen.all().find(n=>n.props.label==='Reset to default')!.props.onPress();await render();
      same((client.getQueryData(key) as any).items.map((i:any)=>i.id),['new_chat']);
      answer(competing);await render();
      same((client.getQueryData(key) as any).items.map((i:any)=>i.id),['new_chat'],'late recovery cannot undo explicit Reset');
      assert.equal(add().props.disabled,false);
    } finally {answer(server);screen.close();client.clear();}
  });
}
