import {it,expect,vi} from 'vitest';
import {render,screen} from '@testing-library/react';
import {CrashReportView} from './admin-crash-reports';
import {crashReportSchema} from '@/schemas/admin-crash-reports';
vi.mock('@/hooks/use-admin-crash-reports',()=>({useCrashSummary:vi.fn(),useCrashReports:vi.fn(),useCrashReport:vi.fn()}));
it('admin report details escape every server string as text',()=>{
 const attack='<img src=x onerror=alert(1)>';
 const report=crashReportSchema.parse({id:attack,event_id:'event',captured_at:'today',platform:'web',kind:'js_error',exception_class:attack,native_signal:null,app_version:'1',app_build:'2',artifact_id:null,os_version:'1',runtime:null,js_frames:[{asset:attack,line:1,column:2}],native_images:[],native_frames:[],environment:attack,severity:'error',fingerprint:attack,received_at:'today',expires_at:'tomorrow',export:{status:'disabled',destination_id:null,attempts:0,last_error_code:null,exported_at:null,provider_event_id:'event'}});
 const {container}=render(<CrashReportView report={report}/>);expect(screen.getAllByText(attack).length).toBeGreaterThan(0);expect(container.querySelector('img')).toBeNull();expect(container.innerHTML).not.toContain('user_id');
});
