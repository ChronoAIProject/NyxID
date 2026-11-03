import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { URL, fileURLToPath } from 'node:url';
import { deferred, hookHarness } from '../../assistant/assistantHookTestHarness';
import type { ViewerState } from './viewer';
import { darkColors, lightColors, type ThemeColors } from '../../../theme/mobileTheme';

type Element = { type: unknown; props: Record<string, any> };
const elements = (root: Element): Element[] => typeof root.type === 'function' ? elements(root.type(root.props)) : [root, ...[root.props.children].flat(Infinity)
  .filter((v): v is Element => !!v && typeof v === 'object' && 'props' in v).flatMap(elements)];
const flattenStyle = (style: unknown): Record<string, any> => Object.assign({}, ...[style].flat(Infinity).filter(value => value && typeof value === 'object'));
function setup(colors: ThemeColors = lightColors) {
  let focused = true;
  let epoch = 0;
  let invalidate: (() => void) | undefined;
  let viewerState: ((value: ViewerState) => void) | undefined;
  let viewerFrame: ((value: unknown) => void) | undefined;
  let alertButtons: { text: string; style?: string; onPress?: () => void }[] = [];
  let inputAccepted = true;
  const inputs: unknown[] = [];
  const calls: string[] = [];
  const orientation = deferred<void>();
  const listeners: ((state:string)=>void)[] = [];
  const appState = { currentState: 'active', addEventListener: (_:string,listener:(state:string)=>void) => { listeners.push(listener);return {remove:()=>{}}; } };
  const jsx=(type:unknown,props:Record<string,unknown>)=>({type,props});
  const workspace = {};
  const harness=hookHarness<Element>({
    'react/jsx-runtime': {jsx,jsxs:jsx,Fragment:'Fragment'},
    'react-native': {View:'View',KeyboardAvoidingView:'KeyboardAvoidingView',Platform:{OS:'ios'},Pressable:'Pressable',Text:'Text',TextInput:'TextInput',StatusBar:'StatusBar',AppState:appState,StyleSheet:{create:(v:unknown)=>v,absoluteFillObject:{}},Keyboard:{dismiss:()=>{}},Alert:{alert:(_title:string,_message:string,buttons:typeof alertButtons)=>{alertButtons=buttons;}}},
    'expo-linear-gradient': {LinearGradient:'LinearGradient'},
    './AndroidKeyboardAvoidingView': {AndroidKeyboardAvoidingView:'AndroidKeyboardAvoidingView'},
    '../../theme/ThemeContext': {useTheme:()=>({colors})},
    '../../../theme/ThemeContext': {useTheme:()=>({colors})},
    '../../../components/workspace/WorkspaceUI': workspace,
    '@react-navigation/native': {useIsFocused:()=>focused},
    'react-native-safe-area-context': {useSafeAreaInsets:()=>({top:0,left:0,right:0,bottom:0})},
    'react-native-webview': {WebView:'WebView'},
    '../../../lib/desktopOrientation': {lockDesktopLandscape:()=>{calls.push('landscape');return orientation.promise;},restorePortrait:async()=>{calls.push('portrait');}},
    '../../../lib/auth/authEpoch': {getAuthEpoch:()=>epoch},
    '../../../lib/auth/sessionStore': {}, '../../../lib/api/http': {}, '../../../lib/env': {},
    './canvasDocument': {canvasDocument:()=>''},
    './compositor': {createCompositor:(frame:(value:unknown)=>void)=>({reset:()=>0,enqueue:()=>frame({generation:0,sequence:'1'}),acknowledge:()=>true})},
    './viewer': {refusalLabel:(e:Error)=>e.message,createDesktopViewer:(deps:{invalidated():void;state(value:ViewerState):void;frame(value:unknown):void})=>{invalidate=deps.invalidated;viewerState=deps.state;viewerFrame=deps.frame;return {start:()=>calls.push('start'),stop:()=>calls.push('stop'),takeControl:()=>calls.push('take-control'),handBack:()=>calls.push('hand-back'),stopTask:()=>calls.push('stop-task'),refresh:()=>calls.push('refresh'),input:(value:unknown)=>{inputs.push(value);return inputAccepted;},fail:()=>{}};}},
  });
  Object.assign(workspace, harness.load(fileURLToPath(new URL('../../../components/workspace/WorkspaceUI.tsx',import.meta.url))));
  const Screen=harness.load(fileURLToPath(new URL('./MachineDesktopScreen.tsx',import.meta.url))).MachineDesktopScreen as (p:unknown)=>Element;
  harness.mount(()=>Screen({route:{params:{nodeId:'node'}},navigation:{goBack:()=>calls.push('close')},privacyProtected:true}));
  const ready=()=>{elements(harness.value()).find(e=>e.type==='WebView')!.props.onMessage({nativeEvent:{data:'{"type":"ready"}'}});harness.flush();};
  const message=(value:unknown)=>{elements(harness.value()).find(e=>e.type==='WebView')!.props.onMessage({nativeEvent:{data:JSON.stringify(value)}});harness.flush();};
  return {harness,calls,orientation,ready,inputs,message,alertButtons:()=>alertButtons,pixels:()=>{viewerFrame?.({});message({type:'decoded',generation:0,sequence:'1',ok:true});},rejectInput:()=>{inputAccepted=false;},acceptInput:()=>{inputAccepted=true;},state:(value:ViewerState)=>{viewerState?.(value);harness.flush();},invalidateAuth:()=>{epoch++;invalidate?.();},blur:()=>{focused=false;harness.render();},background:()=>{appState.currentState='background';listeners.forEach(fn=>fn('background'));harness.flush();}};
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

test('rejected text stays editable with feedback and only an accepted send clears it', async () => {
  const f = setup();
  f.orientation.resolve(); await f.harness.settle(); f.ready();
  f.state({ phase: 'attention', controls: true, label: 'You are controlling this desktop' });
  const button = (label: string) => elements(f.harness.value()).find(e => e.props.accessibilityLabel === label)!;
  button('Keyboard').props.onPress(); f.harness.flush();
  button('Desktop text to send').props.onChangeText('Keep this draft'); f.harness.flush();
  f.rejectInput();
  button('Send').props.onPress(); f.harness.flush();
  assert.equal(button('Desktop text to send').props.value, 'Keep this draft');
  assert.ok(elements(f.harness.value()).some(e => e.type === 'Text' && e.props.children === 'Check control and try again in a moment, or send a shorter message.'));
  f.acceptInput();
  button('Send').props.onPress(); f.harness.flush();
  assert.equal(button('Desktop text to send').props.value, '');
  assert.equal(f.inputs.length, 2);
  assert.ok(elements(f.harness.value()).every(e => e.props.children !== 'Check control and try again in a moment, or send a shorter message.'));
  button('Desktop text to send').props.onChangeText('Private draft'); f.harness.flush();
  f.rejectInput(); button('Send').props.onPress(); f.harness.flush();
  f.state({ phase: 'reconnecting', controls: false, label: 'Reconnecting to the desktop…' });
  assert.ok(elements(f.harness.value()).every(e => e.props.accessibilityLabel !== 'Desktop text to send'));
  f.state({ phase: 'attention', controls: true, label: 'You are controlling this desktop' });
  button('Keyboard').props.onPress(); f.harness.flush();
  assert.equal(button('Desktop text to send').props.value, '', 'losing control clears private text and never restores it for retry');
  assert.equal(f.inputs.length, 3, 'reacquiring control never resends retained text');
  f.harness.unmount();
});

test('shared desktop buttons retain takeover gates, long-press task confirmation and native touch feedback in both themes', async (t) => {
  for (const colors of [lightColors, darkColors]) {
    const f = setup(colors);
    t.after(f.harness.unmount);
    f.orientation.resolve(); await f.harness.settle(); f.ready();
    const button = (label: string) => elements(f.harness.value()).find(e => e.props.accessibilityLabel === label)!;
    const blocked = button('Take control');
    assert.equal(blocked.props.disabled, true);
    assert.equal(blocked.props.accessibilityState.disabled, true);
    assert.deepEqual(flattenStyle(blocked.props.style({ pressed: true })), flattenStyle(blocked.props.style({ pressed: false })));
    assert.equal(button('Keyboard').props.disabled, true);
    const close = button('Close');
    assert.equal(close.props.accessibilityRole, 'button');
    assert.ok(flattenStyle(close.props.style({ pressed: false })).minWidth >= 44);
    const fill = flattenStyle(close.props.children.props.style);
    assert.ok(fill.minHeight >= 44);
    assert.equal(fill.backgroundColor, colors.card);
    assert.equal(fill.borderColor, colors.controlBorder);
    assert.notEqual(flattenStyle(close.props.style({ pressed: true })).opacity, flattenStyle(close.props.style({ pressed: false })).opacity);
    f.pixels();
    assert.equal(!!button('Take control').props.disabled, false);
    button('Take control').props.onPress();
    assert.ok(f.calls.includes('take-control'));
    button('Take control').props.onLongPress();
    assert.equal(f.alertButtons().length, 0, 'task options remain gated by acknowledged control');
    f.state({ phase: 'attention', controls: true, label: 'You are controlling this desktop' });
    const handBack = button('Hand back');
    assert.equal(handBack.props.accessibilityHint, 'Touch and hold for task stop options');
    handBack.props.onLongPress();
    assert.equal(f.calls.includes('stop-task'), false, 'holding opens confirmation without stopping work');
    const stop = f.alertButtons().find(row => row.text === 'Stop task')!;
    assert.equal(stop.style, 'destructive');
    stop.onPress!();
    assert.ok(f.calls.includes('stop-task'));
    handBack.props.onPress();
    assert.ok(f.calls.includes('hand-back'));
    f.state({ phase: 'watching', controls: false, label: 'Watching the desktop' });
    f.message({ type: 'toggleOverlay' });
    const reveal = button('Show desktop controls');
    assert.equal(reveal.props.accessibilityRole, 'button');
    reveal.props.onPress(); f.harness.flush();
    assert.ok(button('Close'));
  }
});

test('rejected desktop text uses the shared error notice over an opaque theme surface while keyboard rows keep wrapping', async (t) => {
  for (const colors of [lightColors, darkColors]) {
    const f = setup(colors);
    t.after(f.harness.unmount);
    f.orientation.resolve(); await f.harness.settle(); f.ready();
    f.state({ phase: 'attention', controls: true, label: 'You are controlling this desktop' });
    const button = (label: string) => elements(f.harness.value()).find(e => e.props.accessibilityLabel === label)!;
    button('Keyboard').props.onPress(); f.harness.flush();
    button('Desktop text to send').props.onChangeText('Retain this text'); f.harness.flush();
    f.rejectInput(); button('Send').props.onPress(); f.harness.flush();
    const tree = elements(f.harness.value());
    const notice = tree.find(e => e.type === 'View' && e.props.accessibilityLiveRegion === 'polite')!;
    assert.equal(flattenStyle(notice.props.style).backgroundColor, colors.dangerTone.bg);
    assert.equal(flattenStyle(notice.props.style).borderColor, colors.dangerTone.border);
    const feedback = tree.find(e => e.type === 'Text' && e.props.children === 'Check control and try again in a moment, or send a shorter message.')!;
    assert.equal(flattenStyle(feedback.props.style).color, colors.dangerTone.text);
    const tray = tree.find(e => e.type === 'View' && flattenStyle(e.props.style).position === 'absolute' && flattenStyle(e.props.style).backgroundColor === colors.card);
    assert.ok(tray, 'translucent notice colors are composed over the current theme card');
    const input = button('Desktop text to send');
    assert.equal(input.props.value, 'Retain this text');
    assert.equal(input.props.secureTextEntry, true);
    assert.equal(flattenStyle(input.props.style).backgroundColor, '#27272a');
    assert.equal(flattenStyle(input.props.style).color, '#fafafa');
    const inputRow = tree.find(e => e.type === 'View' && [e.props.children].flat(Infinity).some(child => child?.type === 'TextInput'))!;
    assert.equal(flattenStyle(inputRow.props.style).flexWrap, 'wrap');
  }
});
