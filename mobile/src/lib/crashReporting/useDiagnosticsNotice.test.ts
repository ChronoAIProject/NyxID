import assert from 'node:assert/strict';
import test from 'node:test';
import {fileURLToPath} from 'node:url';
import {hookHarness} from '../../features/assistant/assistantHookTestHarness';
import {translate} from '../../i18n/translate';
import type {Language} from '../../i18n/language';
const tick=()=>new Promise(resolve=>setImmediate(resolve));
for(const language of ['en','zh-CN','zh-TW'] as Language[])test(`updated notice is account-bound, localized and never changes permission (${language})`,async()=>{
 const stored=new Map<string,string>();const alerts:Array<{title:string;message:string;buttons:Array<{text:string;onPress?:()=>void}>}>=[];let reviews=0;
 const consent={accountId:'a',isHydrated:true,enabled:true,policy:1};let permission={account_id:'a',enabled:true,policy_version:'1',current_policy_version:'2'};
 const t=(phrase:string)=>translate(language,phrase);
 const h=hookHarness<void>({
  'react-native':{Alert:{alert:(title:string,message:string,buttons:Array<{text:string;onPress?:()=>void}>)=>alerts.push({title,message,buttons})}},
  '@react-native-async-storage/async-storage':{getItem:async(k:string)=>stored.get(k)??null,setItem:async(k:string,v:string)=>{stored.set(k,v);}},
  '../consent':{useMobileConsent:()=>consent},
  '../api/http':{requestJson:async()=>permission},
  '../../i18n/LanguageProvider':{useAppLanguage:()=>({t})},
 });
 const {useDiagnosticsNotice}=h.load(fileURLToPath(new URL('./useDiagnosticsNotice.ts',import.meta.url))) as {useDiagnosticsNotice:(loggedIn:boolean,review:()=>void)=>void};
 const review=()=>{reviews++;};h.mount(()=>useDiagnosticsNotice(true,review));await tick();
 assert.equal(alerts.length,1);assert.equal(alerts[0]!.title,t('Analytics updated'));assert.equal(alerts[0]!.buttons[0]!.text,t('Not now'));assert.equal(consent.policy,1);assert.equal(reviews,0);
 alerts[0]!.buttons[1]!.onPress!();assert.equal(reviews,1);
 h.render();await tick();assert.equal(alerts.length,1);
 consent.accountId='b';permission={...permission,account_id:'b'};h.render();await tick();assert.equal(alerts.length,2);h.unmount();
});
