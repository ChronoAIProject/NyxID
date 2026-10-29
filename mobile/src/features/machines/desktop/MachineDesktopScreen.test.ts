import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { URL, fileURLToPath } from 'node:url';
import { deferred, hookHarness } from '../../assistant/assistantHookTestHarness';

type Element = { type: unknown; props: Record<string, any> };
const elements = (root: Element): Element[] => [root, ...[root.props.children].flat(Infinity)
  .filter((v): v is Element => !!v && typeof v === 'object' && 'props' in v).flatMap(elements)];
function setup() {
  let focused = true;
  let epoch = 0;
  let invalidate: (() => void) | undefined;
  const calls: string[] = [];
  const orientation = deferred<void>();
  const listeners: ((state:string)=>void)[] = [];
  const appState = { currentState: 'active', addEventListener: (_:string,listener:(state:string)=>void) => { listeners.push(listener);return {remove:()=>{}}; } };
  const jsx=(type:unknown,props:Record<string,unknown>)=>({type,props});
  const harness=hookHarness<Element>({
    'react/jsx-runtime': {jsx,jsxs:jsx,Fragment:'Fragment'},
    'react-native': {View:'View',KeyboardAvoidingView:'KeyboardAvoidingView',Platform:{OS:'ios'},Pressable:'Pressable',Text:'Text',TextInput:'TextInput',StatusBar:'StatusBar',AppState:appState,StyleSheet:{create:(v:unknown)=>v,absoluteFillObject:{}},Keyboard:{dismiss:()=>{}},Alert:{alert:()=>{}}},
    '@react-navigation/native': {useIsFocused:()=>focused},
    'react-native-safe-area-context': {useSafeAreaInsets:()=>({top:0,left:0,right:0,bottom:0})},
    'react-native-webview': {WebView:'WebView'},
    '../../../lib/desktopOrientation': {lockDesktopLandscape:()=>{calls.push('landscape');return orientation.promise;},restorePortrait:async()=>{calls.push('portrait');}},
    '../../../lib/auth/authEpoch': {getAuthEpoch:()=>epoch},
    '../../../lib/auth/sessionStore': {}, '../../../lib/api/http': {}, '../../../lib/env': {},
    './canvasDocument': {canvasDocument:()=>''},
    './compositor': {createCompositor:()=>({reset:()=>0,enqueue:()=>{},acknowledge:()=>true})},
    './viewer': {refusalLabel:(e:Error)=>e.message,createDesktopViewer:(deps:{invalidated():void})=>{invalidate=deps.invalidated;return {start:()=>calls.push('start'),stop:()=>calls.push('stop'),refresh:()=>{},input:()=>{},fail:()=>{}};}},
  });
  const Screen=harness.load(fileURLToPath(new URL('./MachineDesktopScreen.tsx',import.meta.url))).MachineDesktopScreen as (p:unknown)=>Element;
  harness.mount(()=>Screen({route:{params:{nodeId:'node'}},navigation:{goBack:()=>calls.push('close')},privacyProtected:true}));
  const ready=()=>{elements(harness.value()).find(e=>e.type==='WebView')!.props.onMessage({nativeEvent:{data:'{"type":"ready"}'}});harness.flush();};
  return {harness,calls,orientation,ready,invalidateAuth:()=>{epoch++;invalidate?.();},blur:()=>{focused=false;harness.render();},background:()=>{appState.currentState='background';listeners.forEach(fn=>fn('background'));harness.flush();}};
}
test('screen acquires landscape on mount and restores portrait with teardown on background/blur/unmount',async()=>{
  for(const exit of ['background','blur','unmount'] as const){
    const f=setup();assert.deepEqual(f.calls,['landscape']);f.orientation.resolve();await f.harness.settle();f.ready();assert.ok(f.calls.includes('start'));
    f.calls.length=0;if(exit==='unmount')f.harness.unmount();else f[exit]();
    assert.ok(f.calls.includes('portrait'));assert.ok(f.calls.includes('stop'));
    if(exit!=='unmount')f.harness.unmount();
  }
});
test('account invalidation restores portrait and closes the desktop route',async()=>{
  const f=setup();f.orientation.resolve();await f.harness.settle();f.ready();f.calls.length=0;
  f.invalidateAuth();assert.deepEqual(f.calls,['portrait','close']);f.harness.unmount();
});
test('late landscape completion after exit cannot reopen socket; native orientation failure is visible',async()=>{
  const f=setup();f.ready();f.background();f.orientation.resolve();await f.harness.settle();
  assert.equal(f.calls.includes('start'),false);assert.ok(f.calls.includes('portrait'));f.harness.unmount();
  const failed=setup();failed.orientation.reject(new Error('Landscape is unavailable'));await failed.harness.settle();
  assert.ok(elements(failed.harness.value()).some(e=>e.type==='Text'&&e.props.children==='Landscape is unavailable'));
  assert.ok(elements(failed.harness.value()).some(e=>e.props.accessibilityLabel==='Retry'));failed.harness.unmount();
});
test('desktop has no browser fallback or purchase/navigation escape',()=>{
  const forbidden=['Open','on','the','web'].join(' ');
  for(const file of ['MachineDesktopScreen.tsx','viewer.ts','desktopRoutes.ts']){
    const source=readFileSync(new URL(file,import.meta.url),'utf8');assert.equal(source.includes(forbidden),false);assert.equal(source.includes('openBrowserAsync'),false);
  }
});
