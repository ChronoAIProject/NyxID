// Produce a CRX3 and its pin together. The ephemeral signing key never leaves
// process memory; a release changing the source updates the package and ID pin.
import {generateKeyPairSync,createHash,sign} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../resources/machine-browser');
const archive=spawnSync('python3',['-c',`import io,sys,zipfile,pathlib
out=io.BytesIO()
with zipfile.ZipFile(out,'w',zipfile.ZIP_DEFLATED) as z:
 for p in sorted(pathlib.Path(sys.argv[1]).glob('*.js'))+ [pathlib.Path(sys.argv[1])/'manifest.json']:
  i=zipfile.ZipInfo(p.name,(2026,1,1,0,0,0));i.compress_type=zipfile.ZIP_DEFLATED;i.external_attr=0o100644<<16;z.writestr(i,p.read_bytes())
sys.stdout.buffer.write(out.getvalue())`,root]);
if(archive.status!==0)throw new Error('Could not package extension sources');
const zip=archive.stdout;
const {privateKey,publicKey}=generateKeyPairSync('rsa',{modulusLength:2048});
const der=publicKey.export({type:'spki',format:'der'});
const digest=createHash('sha256').update(der).digest();
const id=digest.subarray(0,16).toString('hex').replace(/[0-9a-f]/g,c=>String.fromCharCode(97+parseInt(c,16)));
const integer=n=>{const b=Buffer.alloc(4);b.writeUInt32LE(n);return b;};
const varint=n=>{const a=[];do{let b=n&127;n>>>=7;if(n)b|=128;a.push(b);}while(n);return Buffer.from(a);};
const field=(number,bytes)=>Buffer.concat([varint((number<<3)|2),varint(bytes.length),bytes]);
const signedHeader=field(1,digest.subarray(0,16));
const signature=sign('sha256',Buffer.concat([Buffer.from('CRX3 SignedData\0'),integer(signedHeader.length),signedHeader,zip]),privateKey);
const header=Buffer.concat([field(2,Buffer.concat([field(1,der),field(2,signature)])),field(10000,signedHeader)]);
const packageBytes=Buffer.concat([Buffer.from('Cr24'),integer(3),integer(header.length),header,zip]);
fs.writeFileSync(path.join(root,'filler.crx'),packageBytes);
fs.writeFileSync(path.join(root,'package.json'),JSON.stringify({extension_id:id,version:JSON.parse(fs.readFileSync(path.join(root,'manifest.json'),'utf8')).version,sha256:createHash('sha256').update(packageBytes).digest('hex')},null,2)+'\n');
console.info(`Packaged extension ${id}`);
