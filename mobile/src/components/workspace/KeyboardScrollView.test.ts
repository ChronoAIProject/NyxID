import assert from 'node:assert/strict';
import test from 'node:test';
import {fileURLToPath} from 'node:url';
import {hookHarness} from '../../features/assistant/assistantHookTestHarness';
import type {KeyboardScrollViewHandle} from './KeyboardScrollView';

for (const reduced of [false, true]) test(`R2-F1 shared viewport reveals an inline banner above the keyboard (reduced motion ${reduced})`, () => {
  const frames = new Map<number, () => void>(); let next = 0;
  const scrolls: Array<{y: number; animated: boolean}> = [];
  let focused: any = {measureInWindow: (cb: any) => cb(0, 150, 300, 40)};
  const controller: {current: KeyboardScrollViewHandle | null} = {current: null};
  const jsx = (type: unknown, props: any) => ({type,props});
  const h = hookHarness<any>({
    'react/jsx-runtime': {jsx},
    'react-native': {ScrollView:'ScrollView', TextInput:{State:{currentlyFocusedInput:()=>focused}}},
    '../../lib/accessibility/useReducedMotion': {useReducedMotion:()=>reduced},
    './KeyboardViewport': {KeyboardViewport:'KeyboardViewport'},
  }, {requestAnimationFrame:(fn:()=>void)=>{frames.set(++next,fn);return next;},cancelAnimationFrame:(id:number)=>frames.delete(id)});
  const {KeyboardScrollView}=h.load(fileURLToPath(new URL('./KeyboardScrollView.tsx',import.meta.url))) as any;
  h.mount(()=>KeyboardScrollView({revealController:controller}));
  const flush=()=>{for(const [id,fn] of frames){frames.delete(id);fn();}};
  try {
    const native=h.value().props.children;
    native.props.ref({scrollTo:(value:any)=>scrolls.push(value)});
    native.props.onFocus({});
    // The shared measured viewport already excludes the open keyboard.
    h.value().props.onViewportChange({y:100,height:400}); flush();
    assert.equal(scrolls.length,0,'focused input is already visible');
    assert.ok(controller.current,'shared reveal controller is attached');
    controller.current.reveal({measureInWindow:(cb:any)=>cb(0,650,300,48)} as any);flush();
    assert.ok(scrolls[0]!.y >= 198,'banner bottom moves above 500pt viewport edge');
    assert.equal(scrolls[0]!.animated,!reduced);
    let late!: (x:number,y:number,width:number,height:number)=>void;
    controller.current.reveal({measureInWindow:(cb:any)=>{late=cb;}} as any);flush();
    // A new focus owns visibility; a delayed old banner measure is fenced.
    focused={measureInWindow:(cb:any)=>cb(0,180,300,40)};
    native.props.onFocus({});flush();late(0,700,300,48);
    assert.equal(scrolls.length,1,'late banner callback cannot move the new input');
  } finally {h.unmount();}
  assert.equal(controller.current,null,'unmounted controller is detached');
});
