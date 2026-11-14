import { beforeEach,afterEach,it,expect,vi } from 'vitest';
import { Component, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { act } from 'react';
import { parseWebStack } from './stack';
import { coarseRuntime } from './runtime';
const s=vi.hoisted(()=>({actor:'account-a',device:{accountId:'account-a',enabled:true,policy:2,revision:0},permission:{account_id:'account-a',enabled:true,policy_version:'2',diagnostics_available:true,revision:'live'},api:vi.fn(),read:vi.fn()}));
vi.mock('@/stores/auth-store',()=>({useAuthStore:{getState:()=>({user:{id:s.actor}})}}));
vi.mock('@/stores/consent-store',()=>({useConsentStore:{getState:()=>s.device}}));
vi.mock('../analytics-consent',()=>({readAnalyticsConsent:s.read}));
vi.mock('../api-client',()=>({apiClient:s.api}));
const sentinel='SECRET_SENTINEL person@example.test Bearer token eyJhbGci.jwt';
function failure(n=1) {const e=new TypeError(sentinel,{cause:new Error(sentinel)});e.stack=`TypeError: ${sentinel}\n at SECRET_FUNCTION (http://localhost:3000/assets/app.js?secret=${encodeURIComponent(sentinel)}#jwt:12:${n})\nSECRET_FUNCTION@https://user:pass@evil.test/private.js:2:3\n at x (file:///Users/SECRET_USER/private.js:1:2)\n at x (data:text/SECRET_DATA:1:2)\n at x (blob:https://secret.test/SECRET_BLOB:1:2)\n at x (chrome-extension://SECRET_EXTENSION/app.js:1:2)`;return e;}
beforeEach(()=>{vi.resetModules();s.actor='account-a';s.device={accountId:'account-a',enabled:true,policy:2,revision:0};s.permission={account_id:'account-a',enabled:true,policy_version:'2',diagnostics_available:true,revision:'live'};s.api.mockReset();s.read.mockReset();s.read.mockImplementation(async()=>({...s.permission}));s.api.mockImplementation(async(_path,options)=>{options.authorityGuard();return {};});Object.defineProperty(navigator,'doNotTrack',{value:'0',configurable:true});Object.defineProperty(navigator,'globalPrivacyControl',{value:false,configurable:true});window.history.replaceState(null,'','/dashboard');});
afterEach(()=>vi.restoreAllMocks());
async function boot(){const reporter=await import('./reporter');await reporter.refreshCrashPermission();return reporter;}
it('stack parsers reconstruct locations and never forward sentinels, URLs, names, messages or causes',()=>{
 const frames=parseWebStack(failure().stack,'http://localhost:3000');
 const bytes=JSON.stringify(frames);
 for(const text of ['SECRET_','person@','Bearer','eyJ','https:','file:','data:','blob:','FUNCTION','/Users'])expect(bytes).not.toContain(text);
 expect(frames[0]?.asset).toBe('assets/app.js');
 expect(parseWebStack(' at fn (http://localhost:3000/assets/main-a.js?token=secret#email:12:4)\nfn@http://localhost:3000/assets/main-a.js:13:5','http://localhost:3000')).toEqual([{asset:'assets/main-a.js',line:12,column:4},{asset:'assets/main-a.js',line:13,column:5}]);
 expect(coarseRuntime('SECRET_USER Mozilla iPhone OS 18_1 Version/18.0 SECRET_TOKEN')).toEqual({os_version:'18.1',runtime:{name:'safari',major:18}});
});
it('request body is closed and each send checks fresh consent and revision',async()=>{
 const r=await boot();await r.reportCrash(failure());expect(s.read).toHaveBeenCalledTimes(2);expect(s.api).toHaveBeenCalledOnce();
 const body=s.api.mock.calls[0]![1].body;expect(body.revision).toBe('live');expect(body.report.app_version).toBeTruthy();expect(body.report.artifact_id).toMatch(/^[a-f0-9-]{36}$/);
 for(const text of ['SECRET_','person@','Bearer','eyJ','https:','/Users','message','cause','stack'])expect(JSON.stringify(body)).not.toContain(text);
});
it('all local gates run before reading stack or making requests',async()=>{
 const r=await boot();const e=new Error();const stack=vi.fn(()=>sentinel);Object.defineProperty(e,'stack',{get:stack});
 for(const mode of ['policy1','off','actor','dnt','gpc','sensitive']) {
   s.device={accountId:'account-a',enabled:mode!=='off',policy:mode==='policy1'?1:2,revision:0};s.actor=mode==='actor'?'b':'account-a';
   Object.defineProperty(navigator,'doNotTrack',{value:mode==='dnt'?'1':'0',configurable:true});Object.defineProperty(navigator,'globalPrivacyControl',{value:mode==='gpc',configurable:true});window.history.replaceState(null,'',mode==='sensitive'?'/login/device':'/dashboard');
   await r.reportCrash(e);
 }
 expect(stack).not.toHaveBeenCalled();expect(s.api).not.toHaveBeenCalled();
});
it('fresh policy denial, withdrawal and changed generations fence pending sends',async()=>{
 const r=await boot();let release!:(p:typeof s.permission)=>void;s.read.mockImplementationOnce(()=>new Promise(resolve=>{release=resolve;}));const pending=r.reportCrash(failure());s.device.revision++;release(s.permission);await pending;expect(s.api).not.toHaveBeenCalled();
 s.permission.policy_version='1';await r.reportCrash(failure(2));expect(s.api).not.toHaveBeenCalled();
});
it('deduplicates the same error across browser and React callbacks and leaves events uncancelled',async()=>{
 await boot();const {installCrashReporting,crashRootOptions}=await import('./install');const remove=installCrashReporting();const e=failure();const event=new ErrorEvent('error',{error:e,cancelable:true});window.dispatchEvent(event);vi.spyOn(console,'error').mockImplementation(()=>{});crashRootOptions.onCaughtError(e);await vi.waitFor(()=>expect(s.api).toHaveBeenCalledOnce());expect(event.defaultPrevented).toBe(false);remove();
});
it('enforces four concurrent admissions and ten reports per page',async()=>{
 const r=await boot();const resolvers:Array<(p:typeof s.permission)=>void>=[];s.read.mockImplementation(()=>new Promise(resolve=>resolvers.push(resolve)));
 const pending=Array.from({length:8},(_,i)=>r.reportCrash(failure(i+1)));expect(resolvers).toHaveLength(4);resolvers.forEach(resolve=>resolve(s.permission));await Promise.all(pending);expect(s.api).toHaveBeenCalledTimes(4);
 s.read.mockResolvedValue(s.permission);for(let i=10;i<24;i++)await r.reportCrash(failure(i));expect(s.api).toHaveBeenCalledTimes(10);
});
it('excludes cancellation, client API refusals and handled chunk recovery',async()=>{
 const r=await boot();const abort=new Error(sentinel);abort.name='AbortError';await r.reportCrash(abort);await r.reportCrash(new (await import('../api-error')).ApiError(403,{error:'forbidden',error_code:1,message:sentinel}));expect(s.api).not.toHaveBeenCalled();
});
it('stops the session after 13400 or 13401',async()=>{
 const r=await boot();s.api.mockRejectedValueOnce(new (await import('../api-error')).ApiError(403,{error:'crash_report_consent_required',error_code:13401,message:sentinel}));await r.reportCrash(failure());await r.refreshCrashPermission();await r.reportCrash(failure(2));expect(s.api).toHaveBeenCalledOnce();
});
it('React caught-error reporting preserves the rendered fallback',async()=>{
 await boot();const {crashRootOptions}=await import('./install');vi.spyOn(console,'error').mockImplementation(()=>{});
 class Boundary extends Component<{children:ReactNode},{failed:boolean}> {state={failed:false};static getDerivedStateFromError(){return{failed:true};}render(){return this.state.failed?<p>Fallback</p>:this.props.children;}}
 function Broken():ReactNode{throw failure();}
 const target=document.createElement('div');const root=createRoot(target,crashRootOptions);await act(async()=>root.render(<Boundary><Broken/></Boundary>));expect(target.textContent).toBe('Fallback');await act(async()=>root.unmount());
});
