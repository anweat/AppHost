// Isolated G2 framework fixtures, not real-project acceptance. Node 24:
// npm ci --ignore-scripts (in this directory), then npm run fixtures.
// Separately: NEXT_TELEMETRY_DISABLED=1 npm run next
// From the repository: node examples/gateway_proxy_probe.mjs 18480 vite=18581 stack=18582 next=18584
// Change vite/version.js or next/app/page.js to observe HMR without navigating.
// /api belongs to this fixture's Vite config; the AppHost probe does not invent it.
import http from 'node:http';
import {createServer} from 'vite';
import {WebSocketServer} from 'ws';
const api = http.createServer((req,res) => {
  if (req.url === '/api/stream') { res.writeHead(200, {'content-type':'text/event-stream','cache-control':'no-store'}); res.write('data: first\n\n'); setTimeout(()=>res.end('data: second\n\n'),1000); }
  else { res.setHeader('content-type','application/json'); res.end(JSON.stringify({host:req.headers.host, origin:req.headers.origin ?? null, forwardedHost:req.headers['x-forwarded-host']})); }
});
const ws = new WebSocketServer({server:api,path:'/api/ws'});
ws.on('connection', socket=>socket.on('message', data=>socket.send(data.toString())));
await new Promise(resolve=>api.listen(18583,'127.0.0.1',resolve));
const vite = await createServer({root:new URL('./vite',import.meta.url).pathname,server:{host:'127.0.0.1',port:18581,strictPort:true}}); await vite.listen();
const stack = await createServer({root:new URL('./stack',import.meta.url).pathname,server:{host:'127.0.0.1',port:18582,strictPort:true,proxy:{'/api':{target:'http://127.0.0.1:18583',ws:true}}}}); await stack.listen();
console.log('fixtures ready');
for (const sig of ['SIGINT','SIGTERM']) process.on(sig,async()=>{for(const c of ws.clients)c.terminate();ws.close();api.closeAllConnections();api.close();await vite.close();await stack.close();});
