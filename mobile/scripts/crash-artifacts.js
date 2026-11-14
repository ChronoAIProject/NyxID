const fs = require('node:fs');
const path = require('node:path');
const {randomUUID} = require('node:crypto');
const {execFileSync} = require('node:child_process');
function prepare(root, platform, version, build) {
  const artifact = randomUUID();
  process.env.NYXID_CRASH_ARTIFACT_ID = artifact;
  const directory = path.join(root,'release','crash-artifacts',platform,`${version}-${build}-${artifact}`);
  fs.mkdirSync(directory,{recursive:true});
  return {directory,app_version:version,app_build:String(build),artifact_id:artifact,platform};
}
function collect(info, bundle, map, dsyms) {
  if(!fs.existsSync(bundle)||!fs.existsSync(map))throw Error('Crash bundle or composed source map missing');
  const name = info.platform==='ios'?'main.jsbundle':'index.android.bundle';
  fs.copyFileSync(bundle,path.join(info.directory,name));
  fs.copyFileSync(map,path.join(info.directory,`${name}.map`));
  const uuids=[];
  if(dsyms) {
    if(!fs.existsSync(dsyms))throw Error('Crash dSYMs missing');
    fs.cpSync(dsyms,path.join(info.directory,'dSYMs'),{recursive:true});
    for(const file of fs.readdirSync(dsyms).filter(f=>f.endsWith('.dSYM'))) {
      const output=execFileSync('dwarfdump',['--uuid',path.join(dsyms,file)],{encoding:'utf8'});
      for(const match of output.matchAll(/UUID: ([A-Fa-f0-9-]+) \(([^)]+)\)/g))uuids.push({file,uuid:match[1],architecture:match[2]});
    }
    if(!uuids.length)throw Error('Crash dSYM UUIDs missing');
  }
  const {directory,...manifest}=info;
  fs.writeFileSync(path.join(directory,'manifest.json'),JSON.stringify({...manifest,bundle:name,map:`${name}.map`,uuids},null,2)+'\n');
  // The uploader reads secrets from its environment and prints no credentials.
  execFileSync(process.execPath,[path.resolve(__dirname,'../../tools/crash-artifacts/upload.mjs'),directory,info.platform],{stdio:'inherit'});
}
module.exports={prepare,collect};
