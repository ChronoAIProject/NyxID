import { Label } from '@/components/ui/label';
import { useState } from 'react';
import { Link,useParams } from '@tanstack/react-router';
import { z } from 'zod';
import { zodResolver } from '@hookform/resolvers/zod';
import { PageHeader } from '@/components/shared/page-header';
import { Button } from '@/components/ui/button';
import { Card,CardContent,CardHeader,CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { useAppForm } from '@/components/ui/form';
import { useUiTranslation,translateUi as t } from '@/i18n/use-ui-translation';
import { useCrashReport,useCrashReports,useCrashSummary } from '@/hooks/use-admin-crash-reports';
import { crashKindSchema,type CrashFilters,type CrashReportDetail } from '@/schemas/admin-crash-reports';
const filterSchema=z.object({platform:z.string(),app_version:z.string().max(32),app_build:z.string().max(64),kind:z.string(),fingerprint:z.string().max(64)});
const outcomeLabels:Record<string,string>={accepted:'Accepted',duplicate:'Duplicate',consent_denied:'Consent denied',unavailable:'Unavailable',rate_limited:'Rate limited',invalid:'Invalid'};
const kindLabels:Record<string,string>={native_crash:'iOS native crash',js_fatal:'JavaScript fatal error',js_error:'JavaScript error',unhandled_rejection:'Unhandled rejection',render_error:'Render error',recoverable_error:'Recoverable error',resource_error:'Resource error'};
const exportLabels:Record<string,string>={disabled:'Disabled',pending:'Pending',dispatching:'Dispatching',exported:'Exported',unknown:'Unknown',failed:'Failed',discarded:'Discarded'};
function label(value:string,labels:Record<string,string>) {return t(labels[value]??value);}
function LoadState({error,loading}:{error:unknown;loading:boolean}) {return error?<p role="alert">{t('Could not load crash reports.')}</p>:loading?<p>{t('Loading…')}</p>:null;}
export function AdminCrashReportsPage() {
  useUiTranslation();
  const [window,setWindow]=useState('24h');
  const [filters,setFilters]=useState<CrashFilters>({});
  const form=useAppForm({resolver:zodResolver(filterSchema),defaultValues:{platform:'',app_version:'',app_build:'',kind:'',fingerprint:''}});
  const summary=useCrashSummary(window,filters.platform??'');const reports=useCrashReports(filters);
  return <div className="space-y-6">
    <PageHeader title={t('Crash reports')} description={t('Reported events by version. Counts reflect consented reports, not all crashes or affected people.')} />
    <form className="flex flex-wrap items-end gap-3" onSubmit={form.handleSubmit(values=>setFilters(values))}>
      <Label>{t('Platform')}<select aria-label={t('Platform')} className="ml-2 rounded-md border border-input bg-background p-2" {...form.register('platform')}><option value="">{t('All platforms')}</option>{['web','ios','android'].map(p=><option key={p} value={p}>{p}</option>)}</select></Label>
      <Label>{t('Version')}<Input aria-label={t('Version')} {...form.register('app_version')} /></Label>
      <Label>{t('Build')}<Input aria-label={t('Build')} {...form.register('app_build')} /></Label>
      <Label>{t('Kind')}<select aria-label={t('Kind')} className="ml-2 rounded-md border border-input bg-background p-2" {...form.register('kind')}><option value="">{t('All kinds')}</option>{crashKindSchema.options.map(k=><option key={k} value={k}>{label(k,kindLabels)}</option>)}</select></Label>
      <Label>{t('Fingerprint')}<Input aria-label={t('Fingerprint')} {...form.register('fingerprint')} /></Label>
      <Button type="submit" variant="outline">{t('Apply filters')}</Button>
      <Label>{t('Window')}<select aria-label={t('Window')} value={window} onChange={e=>setWindow(e.target.value)} className="ml-2 rounded-md border border-input bg-background p-2">{['24h','7d','30d'].map(v=><option key={v}>{v}</option>)}</select></Label>
    </form>
    <LoadState error={summary.error} loading={summary.isLoading}/>
    {summary.data&&<>
      <Card><CardHeader><CardTitle>{t('Ingestion health')}</CardTitle></CardHeader><CardContent className="space-y-4 text-13">
        <dl className="grid grid-cols-2 gap-3 md:grid-cols-6">{Object.entries(outcomeLabels).map(([key,value])=><div key={key}><dt className="text-muted-foreground">{t(value)}</dt><dd className="font-mono text-20">{summary.data.health.outcomes[key]??0}</dd></div>)}</dl>
        <p>{t('Retention')}: {t('{{days}} days',{days:summary.data.health.retention_days})} · {t('Reporting rollout')}: {summary.data.health.flag_enabled?t('Enabled'):t('Disabled')}</p>
        <p>{t('Export destinations')}: {summary.data.health.destination_ids.join(', ')||t('None')}</p>
        <dl className="flex flex-wrap gap-4">{Object.entries(summary.data.health.export_statuses).map(([key,count])=><div key={key}><dt>{label(key,exportLabels)}</dt><dd className="font-mono">{count}</dd></div>)}</dl>
      </CardContent></Card>
      <Card><CardHeader><CardTitle>{t('Reported events by version')}</CardTitle></CardHeader><CardContent className="overflow-x-auto"><table className="w-full text-left text-13"><thead><tr>{['Platform','Environment','Version','Build','Artifact','Reported events','Kind','Last seen'].map(h=><th className="p-2" key={h}>{t(h)}</th>)}</tr></thead><tbody>{summary.data.versions.map((v,i)=><tr key={i} className="border-t border-border"><td className="p-2">{v.platform}</td><td>{v.environment}</td><td><Button variant="link" onClick={()=>{form.setValue('platform',v.platform);form.setValue('app_version',v.app_version);form.setValue('app_build',v.app_build);setFilters({platform:v.platform,app_version:v.app_version,app_build:v.app_build});}}>{v.app_version}</Button></td><td className="font-mono">{v.app_build}</td><td className="font-mono">{v.artifact_id??'—'}</td><td>{v.total}</td><td>{Object.entries(v.counts).filter(([,n])=>n>0).map(([k,n])=>`${label(k,kindLabels)}: ${n}`).join(', ')}</td><td className="font-mono">{v.last_seen}</td></tr>)}</tbody></table></CardContent></Card>
    </>}
    <Card><CardHeader><CardTitle>{t('Individual reports')}</CardTitle></CardHeader><CardContent className="space-y-3 overflow-x-auto"><LoadState error={reports.error} loading={reports.isLoading}/>
      {reports.data?.reports.length===0&&<p>{t('No reports in this selection.')}</p>}
      <table className="w-full text-left text-13"><thead><tr>{['Captured','Platform','Version','Kind','Export status'].map(h=><th className="p-2" key={h}>{t(h)}</th>)}</tr></thead><tbody>{reports.data?.reports.map(r=><tr key={r.id} className="border-t border-border"><td className="p-2 font-mono"><Link to="/admin/crash-reports/$id" params={{id:r.id}} className="underline">{r.captured_at}</Link></td><td>{r.platform}</td><td>{r.app_version} ({r.app_build})</td><td>{label(r.kind,kindLabels)}</td><td>{label(r.export.status,exportLabels)}</td></tr>)}</tbody></table>
      {reports.data?.cursor&&<Button variant="outline" onClick={()=>setFilters({...filters,cursor:reports.data!.cursor!})}>{t('Next page')}</Button>}
      {filters.cursor&&<Button variant="outline" onClick={()=>setFilters({...filters,cursor:undefined})}>{t('First page')}</Button>}
    </CardContent></Card>
  </div>;
}
export function CrashReportView({report:r}:{report:CrashReportDetail}) {
  useUiTranslation();
  return <Card><CardContent className="space-y-4 pt-6 text-13">
    <dl className="grid grid-cols-1 gap-3 sm:grid-cols-2">{Object.entries({[t('Report ID')]:r.id,[t('Platform')]:r.platform,[t('Version')]:r.app_version,[t('Build')]:r.app_build,[t('Artifact')]:r.artifact_id??'—',[t('Environment')]:r.environment,[t('Kind')]:label(r.kind,kindLabels),[t('Exception class')]:r.exception_class,[t('Signal')]:r.native_signal??'—',[t('OS version')]:r.os_version,[t('Runtime')]:r.runtime?`${r.runtime.name} ${r.runtime.major}`:'—',[t('Fingerprint')]:r.fingerprint,[t('Captured')]:r.captured_at,[t('Received')]:r.received_at,[t('Expires')]:r.expires_at,[t('Export status')]:label(r.export.status,exportLabels),[t('Export attempts')]:String(r.export.attempts),[t('Export result code')]:r.export.last_error_code??'—',[t('Provider event ID')]:r.export.provider_event_id}).map(([key,value])=><div key={key}><dt className="text-muted-foreground">{key}</dt><dd className="break-all font-mono">{value}</dd></div>)}</dl>
    <h2 className="font-medium">{t('JavaScript frames')}</h2><ol className="font-mono">{r.js_frames.map((f,i)=><li key={i}>{f.asset}:{f.line}:{f.column}</li>)}</ol>
    <h2 className="font-medium">{t('Native images')}</h2><ol className="font-mono">{r.native_images.map((image,i)=><li key={i}>{i}: {image.name} · {image.uuid} · {image.size}</li>)}</ol>
    <h2 className="font-medium">{t('Native frames')}</h2><ol className="font-mono">{r.native_frames.map((f,i)=><li key={i}>{f.image??'—'} + {f.offset}</li>)}</ol>
    <p className="text-muted-foreground">{t('Frames require the matching private artifacts for symbolication.')}</p>
  </CardContent></Card>;
}
export function AdminCrashReportPage() {
  useUiTranslation();const {id}=useParams({strict:false});const report=useCrashReport(id??'');
  return <div className="space-y-6"><PageHeader title={t('Crash report')} actions={<Link to="/admin/crash-reports">{t('Back to crash reports')}</Link>}/><LoadState error={report.error} loading={report.isLoading}/>{report.data&&<CrashReportView report={report.data}/>}</div>;
}
