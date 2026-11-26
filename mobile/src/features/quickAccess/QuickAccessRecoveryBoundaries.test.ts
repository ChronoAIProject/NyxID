import {setAppTranslationLanguage} from "../../i18n/appTranslation";
import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import {QueryClient, QueryObserver, MutationObserver, onlineManager} from '@tanstack/query-core';
import {hookHarness, deferred} from '../assistant/assistantHookTestHarness';
import {ApiError} from '../../lib/api/ApiError';
import {lightColors} from '../../theme/mobileTheme';
import {quickAccessResponseSchema, QUICK_ACCESS_DEFAULT_STATE} from '../../lib/api/quickAccessSchema';
import {translate} from '../../i18n/translate';
import type {Language} from '../../i18n/language';
const list = (...ids: string[]) => ({customized:true, items:ids.map((id: string)=>({kind:'destination',id}))});
const ids = (state: any) => Array.from(state.items, (item: any)=>item.id);
const conflict = () => new ApiError({statusCode:409,errorCode:1004,errorKey:'conflict',message:'Quick access changed.'});
const turn = () => new Promise(resolve=>setImmediate(resolve));
const key = (epoch: number) => ['account','quick-access',epoch];

function nodes(node: any): any[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!node?.props) return [];
  if (typeof node.type === 'function' && ['AddRow','OrderButton'].includes(node.type.name))
    return nodes(node.type(node.props));
  return [node,...nodes(node.props.children)];
}

function fixture(language: Language='en') {
  setAppTranslationLanguage(language);
  const client = new QueryClient();
  client.mount(); // connect to the installed TanStack onlineManager
  const f: any = {client,server:list('plugins'),epoch:7, actor:'a', gets:[], writes:[],resets:[],screens:[]};
  client.setQueryData(key(f.epoch), f.server);
  const api = {
    get(signal: AbortSignal) {
      const reply=deferred();
      f.gets.push({reply,signal,snapshot:f.server,actor:f.actor});
      return reply.promise;
    },
    async save(items: any[], guard: () => void) {
      guard();
      const reply=deferred<void>();
      f.writes.push({items:Array.from(items),reply});
      await reply.promise;
      f.server={items:Array.from(items),customized:true};
      return f.server;
    },
    async reset(guard: () => void) {
      guard();
      const reply=deferred<void>();
      f.resets.push(reply);
      await reply.promise;
      f.server=QUICK_ACCESS_DEFAULT_STATE;
      return f.server;
    }
  };
  function mount() {
    const queries = new Map<string, QueryObserver<any>>();
    let mutation: any, unsub: any, foreground: any;
    const hooks: any={};
    const jsx=(type: unknown,props: any)=>({type,props});
    const named=(names: string[])=>Object.fromEntries(names.map(n=>[n,n]));
    const t=(phrase: string,values?: any)=>translate(language,phrase,values);
    const h=hookHarness({
      'react/jsx-runtime':{jsx,jsxs:jsx},
      'react-native':{Pressable:'Pressable',View:'View',StyleSheet:{create:(v: unknown)=>v},
        AppState:{addEventListener:(_: string,fn: any)=>{foreground=fn;return {remove(){foreground=undefined}}}}},
      'lucide-react-native':named(['ChevronDown','ChevronUp','Minus','Plus']),
      '../../components/settings/SettingsList':named(['SettingsPage','SettingsGroup','SettingsRow']),
      '../../theme/ThemeContext':{useTheme:()=>({colors:lightColors})},
      '../../i18n/LanguageProvider':{useAppLanguage:()=>({language,t})},
      './QuickAccessGlyph':{QuickAccessGlyph:'QuickAccessGlyph'},
      './useQuickAccess':hooks, './useQuickAccessSources':{useQuickAccessSources:()=>({agents:[],crews:[]})},
      '../../lib/auth/authEpoch':{getAuthEpoch:()=>f.epoch}, '../../lib/auth/authAccount':{getAuthAccountId:()=>f.actor},
      '../../lib/auth/sessionStore':{authSessionGeneration:()=>f.epoch}, '../../lib/api/quickAccessApi':{quickAccessApi:api},
      '@tanstack/react-query':{
        useQueryClient:()=>client, useIsMutating:(opts: any)=>client.isMutating(opts),
        useQuery:(options: any)=>{
          const id=JSON.stringify(options.queryKey);
          if (!queries.has(id)) {
            const q=new QueryObserver(client,options);
            q.subscribe(()=>{});
            queries.set(id,q);
          }
          return queries.get(id)!.getCurrentResult();
        },
        useMutation:(options: any)=>{
          if (!mutation) {
            mutation=new MutationObserver(client,options);
            unsub=mutation.subscribe(()=>{});
          }
          return {...mutation.getCurrentResult(),mutate:(v: unknown,callbacks: any)=>{
            void mutation.mutate(v,callbacks).catch(()=>{});
          },reset:()=>mutation.reset()};
        }
      }
    });
    Object.assign(hooks,h.load(path.resolve('src/features/quickAccess/useQuickAccess.ts')));
    const {QuickAccessSettingsScreen}=h.load(path.resolve('src/features/quickAccess/QuickAccessSettingsScreen.tsx')) as any;
    h.mount(()=>QuickAccessSettingsScreen({}));
    const find=(predicate: (n: any) => boolean)=>nodes(h.value()).find(predicate);
    let closed=false;
    const page={
      render:async()=>{await turn(); h.render(); await h.settle();},
      row:(label: string)=>find(n=>n.props.label===label || n.props.accessibilityLabel===label)?.props,
      footer:()=>find(n=>n.props.title===t('Your shortcuts')).props.footer,
      query:()=>queries.get(JSON.stringify(key(f.epoch))),
      foreground:()=>foreground('active'),
      close:()=>{
        if (closed) return;
        closed=true;h.unmount();queries.forEach(q=>q.destroy());unsub?.();
      }
    };
    f.screens.push(page);
    return page;
  }
  async function provoke(page: any) {
    page.row(translate(language,'Add {{name}}',{name:translate(language,'Machines')})).onPress();
    await page.render();
    f.server=list('plugins','settings');
    f.writes.at(-1).reply.reject(conflict());
    await page.render();
    assert.ok(f.gets.length>0);
  }
  function close() {
    f.screens.forEach((p: any)=>p.close()); client.unmount();client.clear();onlineManager.setOnline(true);setAppTranslationLanguage('en');
  }
  return Object.assign(f,{mount,provoke,close});
}

test('R1-D-a Reset reply keeps ownership when a reconnect GET started during the pending Reset returns late',async()=>{
  const f=fixture(), p=f.mount();
  const now=Date.now;
  try {
    await f.provoke(p);
    p.row('Reset to default').onPress();
    await p.render();
    assert.equal(f.resets.length,1);
    assert.equal(f.gets[0].signal.aborted,true,'Reset cancels the earlier recovery GET');
    const count=f.gets.length;
    // A mobile connection drops during DELETE. A minute later the query is stale on reconnect.
    onlineManager.setOnline(false);
    Date.now=()=>now()+120000;
    onlineManager.setOnline(true);
    await p.render();
    const read=f.gets.length>count ? f.gets.at(-1) : null;
    f.resets[0].resolve();
    await p.render();
    assert.deepEqual(ids(f.client.getQueryData(key(7))),ids(QUICK_ACCESS_DEFAULT_STATE));
    if (read) read.reply.resolve(read.snapshot);
    await p.render();
    const cached=ids(f.client.getQueryData(key(7)));
    console.log('after late reconnect GET, server:',ids(f.server),'editor:',cached);
    p.row('Add Machines').onPress();await p.render();
    f.writes.at(-1).reply.resolve();await p.render();
    const actual=ids(f.server);
    console.log('first ordinary PUT after Reset:',actual);
    assert.deepEqual(actual,[...ids(QUICK_ACCESS_DEFAULT_STATE),'machines']);
  } finally {Date.now=now;f.close();}
});

test('R1-D-a a held then failed Reset keeps recovery fenced until a new authoritative GET succeeds',async()=>{
  const f=fixture(), p=f.mount(), now=Date.now;
  try {
    await f.provoke(p);
    p.row('Reset to default').onPress();await p.render();
    const reads=f.gets.length;
    onlineManager.setOnline(false);
    Date.now=()=>now()+120000;
    onlineManager.setOnline(true);await p.render();
    assert.equal(f.gets.length,reads,'a reconnect must not start a server snapshot while DELETE is held');
    assert.deepEqual(ids(f.client.getQueryData(key(7))),ids(QUICK_ACCESS_DEFAULT_STATE));
    assert.equal(p.row('Add Machines').disabled,true);
    assert.doesNotMatch(p.footer(),/latest list is shown/);
    f.resets[0].reject(new ApiError({statusCode:503,errorCode:1000,errorKey:'unavailable',message:'temporarily unavailable'}));
    await p.render();
    assert.equal(f.gets.length,reads+1,'failed DELETE releases the write fence for a recovery GET');
    assert.equal(p.row('Add Machines').disabled,true);
    f.gets.at(-1).reply.reject(new Error('unreadable recovery response'));await p.render();
    assert.match(p.footer(),/Couldn't reload/);
    p.close();const remounted=f.mount();await remounted.render();
    assert.equal(remounted.row('Add Machines').disabled,true);
    f.gets.at(-1).reply.resolve(f.server);await remounted.render();
    assert.deepEqual(ids(f.client.getQueryData(key(7))),['plugins','settings']);
    assert.equal(remounted.row('Add Machines').disabled,false);
    assert.match(remounted.footer(),/latest list is shown/);
  } finally {Date.now=now;f.close();}
});

for (const language of ['en','zh-CN','zh-TW'] as const) test(`R1-D-b later 503 feedback follows a recovered conflict and successful edit (${language})`,async()=>{
  const f=fixture(language), p=f.mount();
  const t=(phrase: string)=>translate(language,phrase);
  try {
    await f.provoke(p);
    f.gets.at(-1).reply.resolve(f.server);await p.render();
    assert.equal(p.footer(),t('Your shortcuts changed on another device. The latest list is shown. Try again.'));
    p.row(t('Add {{name}}').replace('{{name}}',t('Machines'))).onPress();await p.render();
    f.writes.at(-1).reply.resolve();await p.render();
    p.row(translate(language,'Add {{name}}',{name:t('Notifications')})).onPress();await p.render();
    f.writes.at(-1).reply.reject(new ApiError({statusCode:503,errorCode:1000,errorKey:'unavailable',message:'temporarily unavailable'}));
    await p.render();
    console.log('later 503 footer:',p.footer(),'cache:',ids(f.client.getQueryData(key(7))));
    assert.equal(p.footer(),t("Couldn't save quick access. Your previous list is back."));
  } finally {f.close();}
});

for (const failure of [false,true]) {
  test('recovery stays fenced through remount and ' + (failure?'a failed GET':'a delayed GET') + '; the next edit uses the fresh list',async()=>{
    const f=fixture();
    let p=f.mount();
    try {
      await f.provoke(p);
      const preRemount=f.gets.at(-1);
      if (failure) {preRemount.reply.reject(new Error('unreadable recovery response'));await p.render();}
      p.close();p=f.mount();await p.render();
      assert.equal(p.row('Add Machines').disabled,true);
      assert.doesNotMatch(p.footer(),/latest list is shown/i);
      if (!failure && f.gets.at(-1)!==preRemount) {
        preRemount.reply.resolve(list('plugins'));await p.render();
        assert.equal(p.row('Add Machines').disabled,true);
      }
      p.row('Add Machines').onPress();await p.render();
      assert.equal(f.writes.length,1);
      if (failure) {
        f.gets.at(-1).reply.reject(new Error('unreadable recovery response'));await p.render();
        assert.match(p.footer(),/Couldn't reload/);
        const reads=f.gets.length;
        p.row('Reload shortcuts').onPress();await p.render();
        assert.equal(f.gets.length,reads+1);
      }
      f.gets.at(-1).reply.resolve(f.server);await p.render();
      assert.equal(p.row('Add Machines').disabled,false);
      assert.match(p.footer(),/latest list is shown/);
      p.row('Add Machines').onPress();await p.render();
      f.writes.at(-1).reply.resolve();await p.render();
      assert.deepEqual(ids(f.server),['plugins','settings','machines']);
    } finally {f.close();}
  });
}

test('an authoritative partial list stays read-only; explicit Reset restores defaults',async()=>{
  const f=fixture(), p=f.mount();
  try {
    await f.provoke(p);
    const partial=quickAccessResponseSchema.parse({customized:true,
      items:[{kind:'destination',id:'plugins'},{kind:'future-entry',id:'v-next'}]});
    assert.equal(partial.hasUnsupportedEntries,true);
    f.gets.at(-1).reply.resolve(partial);await p.render();
    assert.match(p.footer(),/need a newer version/);
    assert.equal(p.row('Add Machines').disabled,true);
    p.row('Add Machines').onPress();await p.render();
    assert.equal(f.writes.length,1);
    assert.equal(p.row('Reset to default').disabled,false);
    p.row('Reset to default').onPress();await p.render();
    f.resets[0].resolve();await p.render();
    assert.deepEqual(ids(f.server),ids(QUICK_ACCESS_DEFAULT_STATE));
    assert.equal(p.row('Add Machines').disabled,false);
  } finally {f.close();}
});

test('a late read for account A cannot open or modify account B’s conflict recovery',async()=>{
  const f=fixture();
  let p=f.mount();
  try {
    await f.provoke(p);
    const oldRead=f.gets.at(-1);
    p.close(); f.client.clear();
    f.epoch++; f.actor='b'; f.server=list('activity');
    f.client.setQueryData(key(f.epoch),f.server);
    p=f.mount();await p.render();
    assert.equal(p.row('Add Machines').disabled,false);
    p.row('Add Machines').onPress();await p.render();
    f.server=list('activity','settings');
    f.writes.at(-1).reply.reject(conflict());await p.render();
    assert.equal(p.row('Add Machines').disabled,true);
    oldRead.reply.resolve(list('plugins'));await p.render();
    assert.equal(p.row('Add Machines').disabled,true);
    assert.deepEqual(ids(f.client.getQueryData(key(8))),['activity']);
    f.gets.at(-1).reply.resolve(f.server);await p.render();
    assert.equal(p.row('Add Machines').disabled,false);
    assert.deepEqual(ids(f.client.getQueryData(key(8))),['activity','settings']);
  } finally {f.close();}
});

test('foreground, reconnect, manual read and a late cancelled response preserve an admitted ordinary save',async()=>{
  const f=fixture(), p=f.mount(), now=Date.now;
  try {
    const before=p.query().refetch();
    const earlier=f.gets.at(-1);
    p.row('Add Machines').onPress();await p.render();
    assert.ok(earlier.signal.aborted);
    Date.now=()=>now()+120000;
    p.foreground();
    onlineManager.setOnline(false);onlineManager.setOnline(true);await p.render();
    await p.query().refetch();
    assert.equal(f.gets.length,1,'the competing reads did not issue another GET during the PUT');
    assert.deepEqual(ids(f.client.getQueryData(key(7))),['plugins','machines']);
    f.writes.at(-1).reply.resolve();await p.render();
    earlier.reply.resolve(earlier.snapshot);await before;await p.render();
    assert.deepEqual(ids(f.client.getQueryData(key(7))),['plugins','machines']);
    const after=p.query().refetch();
    assert.equal(f.gets.length,2);
    f.gets.at(-1).reply.resolve(f.server);await after;await p.render();
    assert.deepEqual(ids(f.client.getQueryData(key(7))),['plugins','machines']);
  } finally {Date.now=now;f.close();}
});
