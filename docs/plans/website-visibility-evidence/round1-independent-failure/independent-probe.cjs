const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),crypto=require('node:crypto');
const repo='/Users/huangzongzhe/workspace/vibeCoding/nyxid';
const audit=__dirname;
const req=require('node:module').createRequire(repo+'/frontend/package.json');
const ts=req('typescript'),cache=new Map();
function load(file) {
 file=path.resolve(file); if(cache.has(file)) return cache.get(file);
 if(file.endsWith('.json')) return JSON.parse(fs.readFileSync(file,'utf8'));
 const raw=fs.readFileSync(file,'utf8');
 const output=ts.transpileModule(raw.replaceAll('import.meta.env','({})'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText;
 const module={exports:{}};cache.set(file,module.exports);
 const local=(name)=>{if(!name.startsWith('.'))throw Error('unexpected import '+name);const target=path.resolve(path.dirname(file),name);return load(path.extname(target)?target:target+'.ts');};
 vm.runInNewContext('(function(require,module,exports){'+output+'\n})',{URL,URLSearchParams,console},{filename:file})(local,module,module.exports);
 cache.set(file,module.exports);return module.exports;
}
const base=repo+'/frontend/src/features/website-a/';
const cfg=load(base+'release-config.ts');
const statusCases=['https://nyx.chrono-ai.fun/','https://nyx.chrono-ai.fun/zh','https://nyx.chrono-ai.fun/zh/'];
const status=statusCases.map(url=>{try {return {url,accepted:true,result:cfg.resolveWebsiteRelease({VITE_WEBSITE_STATUS_ENABLED:'true',VITE_WEBSITE_STATUS_URL:url,VITE_WEBSITE_STATUS_ACCEPTANCE:'review/status'}).footerStatus};}catch(e){return {url,accepted:false,error:e.message};}});
const release=cfg.resolveWebsiteRelease({VITE_WEBSITE_SOLUTION_ACTIVATIONS:JSON.stringify({'help-desk':{target:'/assistant?setup=help-desk',acceptance:'review/help-desk'}})});
const auth=load(base+'auth-links.ts');
const sourceQuery='?code=NYX-ABCDEF12';
const targetHref=auth.siteHref(release.solutions['help-desk'],sourceQuery);
const destination=new URL(targetHref,'https://nyx.chrono-ai.fun');
const appSource=fs.readFileSync(repo+'/frontend/src/app-entry.tsx','utf8');
const appExpression=appSource.match(/search: \{ return_to: (window\.location\.origin \+ path \+ window\.location\.search) \}/)[1];
const returnTo=vm.runInNewContext(appExpression,{window:{location:destination},path:destination.pathname});
const loginQuery=new URLSearchParams({return_to:returnTo});
const inviteExpression=fs.readFileSync(repo+'/frontend/src/pages/login.tsx','utf8').match(/const inviteCode = (params\.get\("code"\) \?\? undefined);/)[1];
const inviteCode=vm.runInNewContext(inviteExpression,{params:loginQuery});
const solution={source:'/solutions/help-desk'+sourceQuery,configuredTarget:release.solutions['help-desk'],actualSiteHref:targetHref,appEntryReturnExpression:appExpression,loginHref:'/login?'+loginQuery.toString(),loginInviteExpression:inviteExpression,loginInviteCode:inviteCode??null,normalRegisterHref:auth.authHref('register',sourceQuery),scope:'Pure source-function/expression probe; no browser, auth session, or backend was executed.'};
const boundary=load(repo+'/frontend/src/features/public-site/configuration-boundary.ts');
const first=cfg.canonicalWebsiteConfiguration(release),second=cfg.canonicalWebsiteConfiguration({...release,solutions:{'help-desk':'/dashboard?setup=help-desk'}});
let mismatchRejected=false;try{boundary.assertPublicConfiguration({dataset:{publicRender:'v1'},getAttribute:()=>encodeURIComponent(first)},second)}catch(e){mismatchRejected=/mismatch/.test(e.message)}
(async()=>{
 const {retainedDisabledFeatureCode}=await import(repo+'/frontend/scripts/public-visibility-contract.mjs');
 const literalGate=['{"data-website-feature":"newsletter"}','{"data-website-feature":`newsletter`}'].map(code=>({code,detected:retainedDisabledFeatureCode(code,{newsletter:false,contactChat:false,footerStatus:false})}));
 const result={method:'Read-only TypeScript transpilation in memory; import.meta.env replaced with empty object for deterministic defaults; actual exported functions invoked. App login handoff evaluated from exact source expressions. No build/server/browser.',sourceCommit:'ed91d302ff560388315b6cc1149491bb9d5beb95',status,solution,sameBooleansDifferentTargetRejected:mismatchRejected,literalGate};
 fs.writeFileSync(path.join(audit,'independent-probe.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result,null,2));
})().catch(e=>{console.error(e);process.exitCode=1});
