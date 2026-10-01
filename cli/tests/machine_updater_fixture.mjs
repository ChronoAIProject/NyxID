// Loopback protocol fixture. Secrets are generated in memory and never logged.
import http from 'node:http';
import {createHash,createHmac,randomBytes,randomUUID} from 'node:crypto';
import {createRequire} from 'node:module';
const {WebSocketServer}=createRequire(import.meta.url)('ws');
const key=randomBytes(32), id=randomUUID(), token=`nyx_nauth_${randomBytes(32).toString('hex')}`;
let socket, machine, registrations=0, connections=0, cookieVisits=0, pageVisits=0;
const replies=new Map();
const hash=value=>createHash('sha256').update(value).digest();
function canonical(v){if(Array.isArray(v))return `[${v.map(canonical).join(',')}]`;if(v&&typeof v==='object')return `{${Object.keys(v).sort().map(k=>`${JSON.stringify(k)}:${canonical(v[k])}`).join(',')}}`;return JSON.stringify(v);}
const server=http.createServer(async(req,res)=>{
 res.setHeader('content-type','application/json');
 if(req.url==='/health'){res.end(JSON.stringify({id,registrations,connections,machine,cookieVisits,pageVisits}));return;}
 if(req.url==='/page'){
  pageVisits++;
  if(req.headers.cookie?.includes('nyxid_fixture=retained'))cookieVisits++;
  res.setHeader('set-cookie','nyxid_fixture=retained; Max-Age=3600; SameSite=Lax');res.setHeader('content-type','text/html');
  res.end('<title>Migration fixture</title><h1>Identity and profile retained</h1><button>Works</button>');return;
 }
 if(req.url!=='/call'||!socket){res.statusCode=503;res.end('{}');return;}
 let input='';for await (const part of req) input+=part;
 const {operation,parameters={}}=JSON.parse(input);
 const r={type:'machine_request',node_id:id,request_id:randomUUID(),operation,parameters,timestamp:Math.floor(Date.now()/1000),nonce:randomUUID()};
 const time=Buffer.alloc(8);time.writeBigInt64BE(BigInt(r.timestamp));
 const mac=createHmac('sha256',key).update('nyxid.machine.request.v1\0');
 for(const part of [r.request_id,id,JSON.stringify(operation),hash(canonical(parameters)),time,r.nonce]){const b=Buffer.isBuffer(part)?part:Buffer.from(part),n=Buffer.alloc(8);n.writeBigUInt64BE(BigInt(b.length));mac.update(n).update(b);}
 r.signature=mac.digest('hex');
 const timer=setTimeout(()=>{replies.delete(r.request_id);res.statusCode=504;res.end('{}');},45000);
 replies.set(r.request_id,result=>{clearTimeout(timer);res.end(JSON.stringify(result));});socket.send(JSON.stringify(r));
});
const wss=new WebSocketServer({server});
wss.on('connection',ws=>ws.on('message',(bytes,binary)=>{
 if(binary)return;
 const m=JSON.parse(bytes);
 if(m.type==='register'){registrations++;ws.send(JSON.stringify({type:'register_ok',node_id:id,auth_token:token,signing_secret:key.toString('hex')}));}
 if(m.type==='auth'){
  if(m.node_id!==id||m.token!==token&&m.auth_token!==token){ws.close();return;}
  connections++;socket=ws;machine=null;ws.send(JSON.stringify({type:'auth_ok',heartbeat_interval_secs:3,capabilities:{proxy_binary_chunks:true}}));
 }
 if(m.capabilities?.machine)machine=m.capabilities.machine;
 if(m.type==='machine_result'){replies.get(m.request_id)?.(m.result);replies.delete(m.request_id);}
}));
setInterval(()=>socket?.readyState===1&&socket.send(JSON.stringify({type:'heartbeat_ping'})),1000);
server.listen(33443,'127.0.0.1');
