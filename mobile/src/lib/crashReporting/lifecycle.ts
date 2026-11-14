import type { ConsentSnapshot } from '../analyticsConsentState';
import type { AnalyticsPermission, RelayRequest } from '../analyticsRelay';
import { revalidateNativeRecord, type CrashBinding } from './nativeRecord';
import { createCrashRelay } from './reporter';
import { exceptionClass, validateCrashRecord, type CrashKind, type CrashReport } from '../../../../shared/crash-report';
import { parseMobileStack } from './stack';
export interface CrashNativeModule {
  getAppIdentity(): {version:string;build:string;osVersion?:string}; getBinding(): CrashBinding|null;
  arm(binding: CrashBinding): void; disarm(): void; recordFatalJs(json:string): void; collectPending(): string[]; removeRecords(ids:string[]):void;
}
export function createCrashLifecycle(deps: {native:CrashNativeModule;snapshot:()=>ConsentSnapshot;request:RelayRequest;uuid:()=>string;platform:'ios'|'android';osVersion:()=>string;artifactId?:string}) {
  let binding: CrashBinding|null = null, generation = 0, ready = false, restoring = true;
  const seen = new WeakSet<object>(); let count = 0;
  const eligible = () => { const s=deps.snapshot(); return !restoring && s.isHydrated && s.enabled && s.policy === 2 && !!s.accountId; };
  const disarm = () => { generation++; binding=null;ready=false;deps.native.disarm(); };
  const current = (b:CrashBinding) => eligible() && ready && binding?.epochId===b.epochId && binding.revision===b.revision && deps.snapshot().accountId===b.accountId;
  const send=createCrashRelay(deps.request,current,disarm);
  const makeRecord=(error:unknown,kind:CrashKind):CrashReport|null => {
    if (!binding || !current(binding) || count>=10) return null;
    if (error instanceof Error && ['AbortError','CanceledError','CancelledError'].includes(error.name)) return null;
    if (error && typeof error === 'object') {if(seen.has(error))return null;seen.add(error);}
    const identity=deps.native.getAppIdentity();
    const record=validateCrashRecord({schema_version:1,event_id:deps.uuid(),captured_at:new Date().toISOString(),platform:deps.platform,kind,exception_class:exceptionClass(error),app_version:identity.version,app_build:identity.build,...(deps.artifactId?{artifact_id:deps.artifactId}:{}),os_version:deps.osVersion(),runtime:{name:'hermes',major:0},js_frames:parseMobileStack(error instanceof Error?error.stack:''),native_images:[],native_frames:[]});
    if (record) count++; return record;
  };
  return {
    disarm,
    setRestoring(value:boolean) { restoring=value; if (!value) return this.refresh(); },
    async refresh() {
      generation++;ready=false;
      if (restoring || !deps.snapshot().isHydrated) return;
      if (!eligible()) {disarm();return;}
      const start=deps.snapshot(), epoch=generation;
      // Invalidate a known predecessor before awaiting any network operation.
      const stored=deps.native.getBinding();
      if (stored && stored.accountId!==start.accountId) deps.native.disarm();
      const guard=()=>{const now=deps.snapshot();if(epoch!==generation || now.revision!==start.revision || now.accountId!==start.accountId || !eligible())throw new Error('crash_authority_changed');};
      try {
        const permission=await deps.request<AnalyticsPermission>('/users/me/preferences/analytics',{expectedAccountId:start.accountId!,authorityGuard:guard});guard();
        if (!permission.enabled || permission.policy_version!=='2' || !permission.diagnostics_available || permission.account_id!==start.accountId) {disarm();return;}
        const previous=deps.native.getBinding();
        if (previous?.accountId===start.accountId && previous.revision===permission.revision) binding=previous;
        else {deps.native.disarm();binding={accountId:start.accountId!,revision:permission.revision,epochId:deps.uuid()};}
        deps.native.arm(binding);
        ready=true;
        for (const raw of deps.native.collectPending()) {
          guard(); const record=revalidateNativeRecord(raw);
          if (!record || record.report.platform !== deps.platform || !binding || record.binding.epochId!==binding.epochId || record.binding.accountId!==binding.accountId || record.binding.revision!==binding.revision) {deps.native.disarm();ready=false;binding=null;return;}
          if (await send(record.report,binding)) deps.native.removeRecords([record.report.event_id]);
        }
      } catch { if (epoch===generation) disarm(); }
    },
    fatal(error:unknown) {
      try {const record=makeRecord(error,'js_fatal');if(record)deps.native.recordFatalJs(JSON.stringify(record));}catch{/* Never interfere with RN's fatal handler. */}
    },
    async capture(error:unknown,kind:CrashKind='js_error') {
      try {const record=makeRecord(error,kind);const b=binding;if(record&&b)await send(record,b);}catch{/* Optional diagnostics fail closed. */}
    },
  };
}
