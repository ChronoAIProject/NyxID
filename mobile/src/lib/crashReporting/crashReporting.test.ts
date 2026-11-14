import assert from 'node:assert/strict';
import test from 'node:test';
import {randomUUID} from 'node:crypto';
import {parseMobileStack} from './stack';
import {revalidateNativeRecord,type CrashBinding} from './nativeRecord';
import {chainGlobalHandler,chainRejections,type ErrorHandler} from './globalHandler';
import {createCrashLifecycle,type CrashNativeModule} from './lifecycle';
import type {ConsentSnapshot} from '../analyticsConsentState';
import type {RelayRequest,AnalyticsPermission} from '../analyticsRelay';
const sentinel='SECRET_SENTINEL Bearer private person@example.test eyJhbGci.jwt';
const report=()=>({schema_version:1,event_id:randomUUID(),captured_at:new Date().toISOString(),platform:'ios',kind:'js_fatal',exception_class:'TypeError',app_version:'1.2.3',app_build:'123',os_version:'18.1',runtime:{name:'hermes',major:0},js_frames:[{asset:'main.jsbundle',line:12,column:3}],native_images:[],native_frames:[]});
const bound=():CrashBinding=>({accountId:randomUUID(),revision:randomUUID(),epochId:randomUUID()});
test('Hermes and JSC parser never retains message, cause, function, token, email, URL or absolute path',()=>{
 const frames=parseMobileStack(`Error ${sentinel}\n at SECRET_FUNCTION (address at /Users/SECRET_USER/app/main.jsbundle:12:3)\nSECRET_FUNCTION@https://user:pass@evil.test/SECRET_URL?token=x:2:4\n at x (index.android.bundle:1:2)`);
 assert.deepEqual(frames,[{asset:'main.jsbundle',line:12,column:3},{asset:'<external>',line:2,column:4},{asset:'index.android.bundle',line:1,column:2}]);
 for(const value of ['SECRET_','Bearer','person@','eyJ','/Users','https:'])assert.ok(!JSON.stringify(frames).includes(value));
});
test('native records reject reason registers message customData absolute paths and extra keys at every boundary',()=>{
 const good={binding:bound(),report:report()};assert.ok(revalidateNativeRecord(JSON.stringify(good)));
 for(const key of ['reason','registers','message','customData','absolutePath','extra']) {
   assert.equal(revalidateNativeRecord(JSON.stringify({...good,[key]:sentinel})),null);
   assert.equal(revalidateNativeRecord(JSON.stringify({...good,report:{...good.report,[key]:sentinel}})),null);
   assert.equal(revalidateNativeRecord(JSON.stringify({...good,binding:{...good.binding,[key]:sentinel}})),null);
 }
 assert.equal(revalidateNativeRecord(JSON.stringify({...good,report:{...good.report,js_frames:[{asset:'/Users/SECRET_USER/main.jsbundle',line:1,column:2}]}})),null);
 assert.equal(revalidateNativeRecord(JSON.stringify({...good,report:{...good.report,runtime:{name:'hermes',major:0,message:sentinel}}})),null);
});
test('fatal handler persists synchronously before the original RN fatal handler and preserves rejection callbacks',()=>{
 const calls:string[]=[];let handler:ErrorHandler=(_e,fatal)=>{assert.equal(fatal,true);calls.push('previous');};
 const restore=chainGlobalHandler({getGlobalHandler:()=>handler,setGlobalHandler:h=>{handler=h;}},()=>calls.push('persist'),()=>calls.push('nonfatal'));
 handler(new Error(sentinel),true);assert.deepEqual(calls,['persist','previous']);restore();
 const options=chainRejections({onUnhandled:()=>calls.push('rn-unhandled'),onHandled:()=>calls.push('rn-handled')},()=>calls.push('report'));
 options.onUnhandled!(1,new Error(sentinel));options.onHandled!(1);assert.deepEqual(calls.slice(2),['report','rn-unhandled','rn-handled']);
});
function harness() {
 let snapshot:ConsentSnapshot={accountId:randomUUID(),revision:1,enabled:true,asked:true,policy:2,isHydrated:true,persistenceError:false};
 let binding:CrashBinding|null=null,records:string[]=[],permission:AnalyticsPermission={account_id:snapshot.accountId!,revision:randomUUID(),enabled:true,policy_version:'2',current_policy_version:'2',diagnostics_available:true};
 const sent:unknown[]=[];const fatal:string[]=[];let arms=0,purges=0;
 const native:CrashNativeModule={getAppIdentity:()=>({version:'9.8.7',build:'654'}),getBinding:()=>binding,arm:b=>{arms++;binding={...b};},disarm:()=>{purges++;binding=null;records=[];},recordFatalJs:json=>{fatal.push(json);records.push(JSON.stringify({binding,report:JSON.parse(json)}));},collectPending:()=>records,removeRecords:ids=>{records=records.filter(raw=>!ids.includes(JSON.parse(raw).report.event_id));}};
 const request:RelayRequest=async<T>(path:string,opts?:Parameters<RelayRequest>[1])=>{opts?.authorityGuard?.();if(path.endsWith('/analytics'))return permission as T;sent.push(opts?.body);return {} as T;};
 const lifecycle=createCrashLifecycle({native,snapshot:()=>snapshot,request,uuid:randomUUID,platform:'ios',osVersion:()=> '18.1'});
 return {lifecycle,sent,fatal,native,get binding(){return binding;},get records(){return records;},get arms(){return arms;},get purges(){return purges;},snapshot:()=>snapshot,change:(patch:Partial<ConsentSnapshot>)=>{snapshot={...snapshot,...patch,revision:snapshot.revision+1};},permission:(patch:Partial<AnalyticsPermission>)=>{permission={...permission,...patch};}};
}
test('relaunch re-arms the same epoch and drains a closed fatal record with native installed version',async()=>{
 const h=harness();await h.lifecycle.setRestoring(false);const epoch=h.binding!.epochId;
 const error=new TypeError(sentinel,{cause:new Error(sentinel)});error.stack=` at SECRET_FUNCTION (address at /Users/SECRET_USER/main.jsbundle:12:3)`;h.lifecycle.fatal(error);assert.equal(h.fatal.length,1);
 await h.lifecycle.refresh();assert.equal(h.binding!.epochId,epoch);assert.equal(h.arms,2);assert.equal(h.records.length,0);assert.equal(h.sent.length,1);
 const body=h.sent[0] as {report:{app_version:string;app_build:string}};assert.equal(body.report.app_version,'9.8.7');assert.equal(body.report.app_build,'654');
 for(const value of ['SECRET_','Bearer','person@','eyJ','/Users','message','cause'])assert.ok(!JSON.stringify(body).includes(value));
});
test('account A to B to A, withdrawal/regrant, kill switch and logout cannot resurrect records',async()=>{
 const h=harness();await h.lifecycle.setRestoring(false);const a=h.snapshot().accountId!;const first=h.binding!.epochId;
 h.lifecycle.fatal(new Error(sentinel));const b=randomUUID();h.change({accountId:b});h.permission({account_id:b,revision:randomUUID()});await h.lifecycle.refresh();assert.equal(h.records.length,0);
 h.change({accountId:a});h.permission({account_id:a,revision:randomUUID()});await h.lifecycle.refresh();assert.notEqual(h.binding!.epochId,first);assert.equal(h.sent.length,0);
 h.lifecycle.fatal(new Error(sentinel));h.change({enabled:false});await h.lifecycle.refresh();assert.equal(h.binding,null);assert.equal(h.records.length,0);
 h.change({enabled:true});h.permission({revision:randomUUID()});await h.lifecycle.refresh();assert.equal(h.records.length,0);
 h.lifecycle.fatal(new Error(sentinel));h.permission({diagnostics_available:false});await h.lifecycle.refresh();assert.equal(h.binding,null);assert.equal(h.records.length,0);
 h.change({accountId:null});h.lifecycle.disarm();assert.equal(h.binding,null);assert.ok(h.purges>=4);
});
test('policy one and unhydrated choices never arm',async()=>{
 const h=harness();h.change({policy:1});await h.lifecycle.setRestoring(false);assert.equal(h.arms,0);h.change({policy:2,isHydrated:false});await h.lifecycle.refresh();assert.equal(h.arms,0);
});

test('Android uses installed version and reports JavaScript fatal records only',async()=>{
 const b=bound();const snapshot:ConsentSnapshot={accountId:b.accountId,revision:1,enabled:true,asked:true,policy:2,isHydrated:true,persistenceError:false};
 const calls:string[]=[];let saved:Record<string,unknown>|null=null;
 const lifecycle=createCrashLifecycle({native:{getAppIdentity:()=>({version:'2.3.4',build:'99'}),getBinding:()=>b,arm:()=>calls.push('arm'),disarm:()=>calls.push('purge'),recordFatalJs:json=>{saved=JSON.parse(json);},collectPending:()=>[],removeRecords:()=>{}},snapshot:()=>snapshot,request:async<T>()=>({account_id:b.accountId,revision:b.revision,enabled:true,policy_version:'2',diagnostics_available:true}) as T,uuid:randomUUID,platform:'android',osVersion:()=> '35'});
 await lifecycle.setRestoring(false);lifecycle.fatal(new TypeError(sentinel));
 assert.equal((saved as unknown as Record<string,unknown>).platform,'android');assert.equal((saved as unknown as Record<string,unknown>).kind,'js_fatal');assert.equal((saved as unknown as Record<string,unknown>).app_version,'2.3.4');assert.ok(!JSON.stringify(saved).includes('SECRET_SENTINEL'));
});
