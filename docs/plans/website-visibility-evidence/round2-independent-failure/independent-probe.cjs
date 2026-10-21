const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),assert=require('node:assert/strict');
const repo='/Users/huangzongzhe/workspace/vibeCoding/nyxid';
const req=require('node:module').createRequire(repo+'/frontend/package.json'),ts=req('typescript');
const origin='https://nyx.chrono-ai.fun',base=repo+'/frontend/src/features/website-a/';
function loader(runtimeEnv={},window={location:{origin,pathname:'/',search:''}}){
 const cache=new Map();
 function load(file){file=path.resolve(file);if(cache.has(file))return cache.get(file);if(file.endsWith('.json'))return JSON.parse(fs.readFileSync(file,'utf8'));
  const raw=fs.readFileSync(file,'utf8'),output=ts.transpileModule(raw.replaceAll('import.meta.env','runtimeEnv'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,esModuleInterop:true}}).outputText;
  const module={exports:{}};cache.set(file,module.exports);const local=name=>{if(!name.startsWith('.'))throw Error('unexpected import '+name);const target=path.resolve(path.dirname(file),name);return load(path.extname(target)?target:target+'.ts');};
  vm.runInNewContext('(function(require,module,exports){'+output+'\n})',{URL,URLSearchParams,console,runtimeEnv,window},{filename:file})(local,module,module.exports);cache.set(file,module.exports);return module.exports;
 }return load;
}
const cfg=loader()(base+'release-config.ts');
const rejected=['/','/zh','/zh/','/zh//','//zh','/%7A%68','/%7A%68%2f%2f','/zh%2f..%2fzh','/%2fzh','/%invalid'];
const permitted=['https://status.example.test/','https://status.example.test/incidents','https://nyx.chrono-ai.fun/status'];
const status=[...rejected.map(p=>({url:origin+p,expect:false})),...permitted.map(url=>({url,expect:true})),...['/health','/api/status','/%68ealth','//api/status'].map(p=>({url:'https://status.example.test'+p,expect:false}))].map(c=>{let accepted=false,error;try{cfg.resolveWebsiteRelease({VITE_WEBSITE_STATUS_ENABLED:'true',VITE_WEBSITE_STATUS_URL:c.url,VITE_WEBSITE_STATUS_ACCEPTANCE:'review/status'});accepted=true}catch(e){error=e.message}return {...c,accepted,pass:accepted===c.expect,error};});
const target='/assistant?setup=help-desk&draft=hello%20world';
const runtimeEnv={VITE_WEBSITE_SOLUTION_ACTIVATIONS:JSON.stringify({'help-desk':{target,acceptance:'review/help-desk'}})};
const window={location:{origin,pathname:'/solutions/help-desk',search:'?code=NYX-ABCDEF12&return_to=https%3A%2F%2Fevil.test&utm_source=campaign'}};
const load=loader(runtimeEnv,window),auth=load(base+'auth-links.ts'),solutions=load(base+'solution-availability.ts'),configured=load(base+'release-config.ts').websiteRelease.solutions['help-desk'];
const trust=load(repo+'/frontend/src/lib/return-url.ts');
const activeConfig=load(base+'release-config.ts'),boundary=load(repo+'/frontend/src/features/public-site/configuration-boundary.ts');
const firstConfig=activeConfig.canonicalWebsiteConfiguration(activeConfig.websiteRelease),secondConfig=activeConfig.canonicalWebsiteConfiguration({...activeConfig.websiteRelease,solutions:{'help-desk':'/dashboard?setup=help-desk'}});
let sameBooleansDifferentTargetRejected=false;try{boundary.assertPublicConfiguration({dataset:{publicRender:'v1'},getAttribute:()=>encodeURIComponent(firstConfig)},secondConfig)}catch(e){sameBooleansDifferentTargetRejected=/mismatch/.test(e.message)}
const ctaHref=auth.siteHref(auth.solutionAuthPath(configured));
const browserSpec=fs.readFileSync(repo+'/frontend/e2e-public/public-site.spec.ts','utf8');
const browserExpectedExpression=browserSpec.match(/\.toHaveAttribute\("href", (publication\.configuration\.solutions\["help-desk"\])\)/)[1];
const browserExpected=vm.runInNewContext(browserExpectedExpression,{publication:{configuration:{solutions:{'help-desk':configured}}}});
const cleanCtaHref=auth.siteHref(auth.solutionAuthPath(configured),'');
const enabledBrowserAssertion={spec:'frontend/e2e-public/public-site.spec.ts:94',expectedExpression:browserExpectedExpression,expected:browserExpected,actualCtaHref:cleanCtaHref,pass:cleanCtaHref===browserExpected,scope:'Exact expected expression versus actual CTA URL builder; browser was not executed.'};
const fragmentHref=auth.legacyDestination('#/start?t=help-desk',window.location.search);
function inspect(href){const u=new URL(href,origin);return{href,path:u.pathname,code:u.searchParams.get('code'),returnTo:u.searchParams.get('return_to'),trusted:trust.resolveTrustedAuthReturnTo(u.searchParams.get('return_to')??undefined)}}
// Execute the exact existing real /start route's beforeLoad function, extracted by TypeScript AST.
const routerPath=repo+'/frontend/src/router.tsx',routerText=fs.readFileSync(routerPath,'utf8'),ast=ts.createSourceFile(routerPath,routerText,ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX);
let handler;function visit(n){if(ts.isVariableDeclaration(n)&&n.name.getText(ast)==='legacyStartRoute'){const obj=n.initializer.arguments[0];handler=obj.properties.find(p=>p.name?.getText(ast)==='beforeLoad').initializer.getText(ast);}ts.forEachChild(n,visit);}visit(ast);assert(handler);
window.location.pathname='/start';window.location.search='?t=help-desk&code=NYX-ABCDEF12';let replaced;window.location.replace=href=>replaced=href;
vm.runInNewContext('('+handler+')',{window,solutionStartTarget:solutions.solutionStartTarget,siteHref:auth.siteHref,authHref:auth.authHref})();
const workspace=new URL(replaced,origin);
const appSource=fs.readFileSync(repo+'/frontend/src/app-entry.tsx','utf8'),appExpression=appSource.match(/search: \{ return_to: (window\.location\.origin \+ path \+ window\.location\.search) \}/)[1];
const returnTo=vm.runInNewContext(appExpression,{window:{location:workspace},path:workspace.pathname});
const loginParams=new URLSearchParams({return_to:returnTo}),loginExpression=fs.readFileSync(repo+'/frontend/src/pages/login.tsx','utf8').match(/const inviteCode = (params\.get\("code"\) \?\? undefined);/)[1];
const invite=vm.runInNewContext(loginExpression,{params:loginParams});
(async()=>{const {retainedDisabledFeatureCode}=await import(repo+'/frontend/scripts/public-visibility-contract.mjs');
 const names=['newsletter','contact-chat','footer-status'],flags=['newsletter','contactChat','footerStatus'],gate=[];
 for(const keyQuote of ['"',"'"])for(const valueQuote of ['"',"'",'`'])for(let enabled=-1;enabled<3;enabled++){
  const text=names.map(n=>'{'+keyQuote+'data-website-feature'+keyQuote+':'+valueQuote+n+valueQuote+'}').join(';'),visibility=Object.fromEntries(flags.map((f,i)=>[f,i===enabled])),actual=retainedDisabledFeatureCode(text,visibility),expected=names.filter((_,i)=>i!==enabled);gate.push({keyQuote,valueQuote,enabled,actual,expected,pass:JSON.stringify(actual)===JSON.stringify(expected)});
 }
 const negatives=['{"data-website-feature":`newsletter${suffix}`}','{"data-website-feature":"contact-chat-preview"}','{"data-website-feature":`newsletter"}',"{'data-website-feature\":'newsletter'}"];
 const boundaryCases=negatives.map(text=>({text,actual:retainedDisabledFeatureCode(text,{newsletter:false,contactChat:false,footerStatus:false})}));
 const result={sourceCommit:'f70b7c4c75baa0b77646a9fe986866ff67327128',engineeringCommit:'44bde4d23ded1cba2d7747e00bc146d5feaf0c96',method:'Actual TS functions transpiled only in memory, import.meta.env supplied explicitly; real /start beforeLoad extracted via TypeScript AST and invoked with window.location.replace spy. App-entry and login expressions evaluated from exact source. No build, browser, real session or backend.',status,cta:inspect(ctaHref),fragment:inspect(fragmentHref),realStart:{input:origin+'/start?t=help-desk&code=NYX-ABCDEF12',handler,configuredTarget:configured,firstDestination:replaced,loginHref:'/login?'+loginParams,loginCode:invite??null,expectedTopLevelCode:'NYX-ABCDEF12',pass:invite==='NYX-ABCDEF12'},gate,boundaryCases,enabledBrowserAssertion,sameBooleansDifferentTargetRejected};
 fs.writeFileSync(path.join(__dirname,'independent-probe.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({statusCases:status.length,statusAllPass:status.every(c=>c.pass),cta:result.cta,fragment:result.fragment,realStart:result.realStart,gateCases:gate.length,gateAllPass:gate.every(c=>c.pass),boundaryCases,enabledBrowserAssertion,sameBooleansDifferentTargetRejected},null,2));
})().catch(e=>{console.error(e);process.exitCode=1});
