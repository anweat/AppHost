// G2 transport experiment, not the daemon gateway. Never starts an App or reads
// its registry. Usage: node examples/gateway_proxy_probe.mjs 18480 vite=5173
// Keep Host/Origin intact so the experiment exposes real framework restrictions.
import http from 'node:http';
import {pathToFileURL} from 'node:url';

const hop = new Set(['connection', 'keep-alive', 'proxy-authenticate',
  'proxy-authorization', 'te', 'trailer', 'transfer-encoding', 'upgrade']);

function forwarded(req, upgrade = false) {
  const remove = new Set([...hop, ...(req.headers.connection ?? '').toLowerCase().split(',').map(s => s.trim())]);
  const headers = Object.fromEntries(Object.entries(req.headers).filter(([key]) =>
    !remove.has(key) && key !== 'forwarded' && !key.startsWith('x-forwarded-')));
  headers.host = req.headers.host;
  if (req.headers.origin !== undefined) headers.origin = req.headers.origin;
  headers['x-forwarded-host'] = req.headers.host;
  headers['x-forwarded-proto'] = 'http';
  if (upgrade) Object.assign(headers, {connection: 'Upgrade', upgrade: 'websocket'});
  return headers;
}

export function createProbe(routes) {
  if (!(routes instanceof Map) || routes.size < 1 || routes.size > 16) throw Error('Supply 1..16 routes');
  for (const [slug, port] of routes) {
    if (!/^[a-z][a-z0-9-]{0,39}$/.test(slug) || !Number.isInteger(port) || port < 1 || port > 65535) {
      throw Error('Invalid slug or upstream port');
    }
  }
  routes = new Map(routes);
  const sockets = new Set();
  const server = http.createServer({maxHeaderSize: 16384, headersTimeout: 5000, requestTimeout: 30000});
  server.maxHeadersCount = 128;
  server.maxConnections = 64;
  server.on('connection', socket => {
    sockets.add(socket);
    socket.setTimeout(120000, () => socket.destroy());
    socket.on('close', () => sockets.delete(socket));
  });
  const target = req => {
    const hosts = req.rawHeaders.filter((_, i) => i % 2 === 0 && req.rawHeaders[i].toLowerCase() === 'host');
    if (hosts.length !== 1 || req.rawHeaders.length > 256 || !req.url.startsWith('/') ||
        req.url.startsWith('//') || req.url.includes('\\')) return null;
    const suffix = `.localhost:${server.address().port}`;
    const host = req.headers.host;
    if (!host.endsWith(suffix)) return null;
    return routes.get(host.slice(0, -suffix.length)) ?? null;
  };
  server.on('request', (req, res) => {
    const port = target(req);
    if (!port) { res.writeHead(421, {'connection': 'close'}); res.end('Unknown probe route'); return; }
    const upstream = http.request({hostname: '127.0.0.1', port, path: req.url,
      method: req.method, headers: forwarded(req), agent: false}, incoming => {
      const remove = new Set([...hop, ...(incoming.headers.connection ?? '').toLowerCase().split(',').map(s => s.trim())]);
      res.writeHead(incoming.statusCode, Object.fromEntries(Object.entries(incoming.headers).filter(([k]) => !remove.has(k))));
      incoming.on('error', () => res.destroy());
      incoming.pipe(res);
    });
    upstream.setTimeout(120000, () => upstream.destroy());
    upstream.on('error', () => {
      if (res.headersSent) res.destroy();
      else { res.writeHead(502); res.end('Probe upstream unavailable'); }
    });
    req.on('aborted', () => upstream.destroy());
    res.on('close', () => upstream.destroy());
    req.pipe(upstream);
  });
  server.on('upgrade', (req, client, head) => {
    const port = target(req);
    if (!port || req.method !== 'GET' || req.headers.upgrade?.toLowerCase() !== 'websocket') {
      client.end('HTTP/1.1 421 Misdirected Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n'); return;
    }
    const upstream = http.request({hostname: '127.0.0.1', port, path: req.url,
      headers: forwarded(req, true), agent: false});
    upstream.setTimeout(5000, () => upstream.destroy());
    upstream.on('error', () => client.destroy());
    client.on('error', () => upstream.destroy());
    client.on('close', () => upstream.destroy());
    upstream.on('response', incoming => {
      incoming.resume();
      client.end(`HTTP/1.1 ${incoming.statusCode} Upgrade rejected\r\nConnection: close\r\nContent-Length: 0\r\n\r\n`);
    });
    upstream.on('upgrade', (incoming, remote, remoteHead) => {
      sockets.add(remote);
      remote.on('close', () => { sockets.delete(remote); client.destroy(); });
      remote.on('error', () => client.destroy());
      client.on('close', () => remote.destroy());
      remote.setTimeout(120000, () => remote.destroy());
      const headers = incoming.rawHeaders.reduce((s, v, i, a) => i % 2 === 0 ? s + `${v}: ${a[i + 1]}\r\n` : s, '');
      client.write(`HTTP/1.1 101 Switching Protocols\r\n${headers}\r\n`);
      if (remoteHead.length) client.write(remoteHead);
      if (head.length) remote.write(head);
      client.pipe(remote).pipe(client);
    });
    upstream.end();
  });
  server.on('connect', (_, socket) => socket.destroy());
  server.on('clientError', (_, socket) => socket.destroy());
  return {server, close: () => new Promise(resolve => {
    for (const socket of sockets) socket.destroy();
    server.close(resolve);
  })};
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const port = Number(process.argv[2]);
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw Error('Invalid listen port');
  const probe = createProbe(new Map(process.argv.slice(3).map(value => {
    const [slug, upstream] = value.split('='); return [slug, Number(upstream)];
  })));
  probe.server.listen(port, '127.0.0.1', () => console.log(`PORT=${probe.server.address().port}`));
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => probe.close());
}
