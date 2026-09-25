// Executable transport boundaries; these fixtures are not real-App G2 acceptance.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import net from 'node:net';
import {once} from 'node:events';
import {createHash} from 'node:crypto';
import {createProbe} from './gateway_proxy_probe.mjs';

async function fixture(t, handler) {
  const upstream = http.createServer(handler);
  upstream.listen(0, '127.0.0.1'); await once(upstream, 'listening');
  const {server, close} = createProbe(new Map([['probe', upstream.address().port]]));
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  t.after(async () => { await close(); upstream.closeAllConnections(); await new Promise(r => upstream.close(r)); });
  return {port: server.address().port, upstream, server};
}
function request(port, path = '/', extra = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request({hostname: '127.0.0.1', port, path,
      headers: {host: `probe.localhost:${port}`, ...extra}}, res => {
      let body = ''; res.setEncoding('utf8'); res.on('data', b => body += b);
      res.on('end', () => resolve({code: res.statusCode, body}));
    });
    req.on('error', reject); req.end();
  });
}

test('Host/Origin/path survive; untrusted forwarding headers do not', async t => {
  const {port} = await fixture(t, (req, res) => res.end(JSON.stringify({path: req.url, headers: req.headers})));
  const result = await request(port, '/api/value?x=1', {origin: `http://probe.localhost:${port}`,
    'x-forwarded-host': 'attacker.example', 'x-forwarded-for': '1.2.3.4', forwarded: 'host=attacker.example'});
  const data = JSON.parse(result.body);
  assert.equal(data.path, '/api/value?x=1');
  assert.equal(data.headers.host, `probe.localhost:${port}`);
  assert.equal(data.headers.origin, `http://probe.localhost:${port}`);
  assert.equal(data.headers['x-forwarded-host'], data.headers.host);
  assert.equal(data.headers['x-forwarded-proto'], 'http');
  assert.equal(data.headers['x-forwarded-for'], undefined);
  assert.equal(data.headers.forwarded, undefined);
});

test('unknown Host and absolute request target cannot select an upstream', async t => {
  let hits = 0;
  const {port} = await fixture(t, (_, res) => { hits++; res.end('unexpected'); });
  for (const host of ['unknown.localhost', '127.0.0.1', 'probe.localhost.evil', `probe.localhost:${port + 1}`]) {
    assert.equal((await request(port, '/', {host})).code, 421);
  }
  for (const path of ['http://127.0.0.1:1/', '//127.0.0.1:1/', '/\\host']) {
    assert.equal((await request(port, path)).code, 421);
  }
  assert.equal(hits, 0);
});

test('streaming chunk reaches client before upstream completes', async t => {
  let release;
  const {port} = await fixture(t, (_, res) => {
    res.writeHead(200, {'content-type': 'text/event-stream'});
    res.write('data: first\n\n'); release = () => res.end('data: last\n\n');
  });
  const res = await new Promise((resolve, reject) => {
    const req = http.get({hostname: '127.0.0.1', port, headers: {host: `probe.localhost:${port}`}}, resolve);
    req.on('error', reject);
  });
  const [chunk] = await once(res, 'data');
  assert.equal(chunk.toString(), 'data: first\n\n');
  assert.equal(res.complete, false);
  const ended = once(res, 'end'); release(); res.resume(); await ended;
});

test('WebSocket upgrade preserves headers and forwards bytes in both directions', async t => {
  const {port, upstream, server} = await fixture(t, (_, res) => res.end());
  const upgrades = [];
  server.on('probe-upgrade', event => upgrades.push(event));
  upstream.on('upgrade', (req, socket, head) => {
    assert.equal(req.headers.host, `probe.localhost:${port}`);
    assert.equal(req.headers.origin, `http://probe.localhost:${port}`);
    assert.equal(req.headers['sec-websocket-protocol'], 'fixture');
    socket.write('HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n');
    if (head.length) socket.write(head);
    socket.pipe(socket);
  });
  const client = net.connect(port, '127.0.0.1');
  t.after(() => client.destroy());
  await once(client, 'connect');
  client.write(`GET /ws?token=private-fixture-token HTTP/1.1\r\nHost: probe.localhost:${port}\r\nOrigin: http://probe.localhost:${port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Protocol: fixture\r\n\r\n`);
  const [response] = await once(client, 'data');
  assert.match(response.toString(), /^HTTP\/1.1 101 /);
  assert.deepEqual(upgrades, [{host: `probe.localhost:${port}`, path: '/ws'}]);
  const echoed = once(client, 'data'); client.write(Buffer.from([0x81, 0x02, 0x6f, 0x6b]));
  assert.deepEqual((await echoed)[0], Buffer.from([0x81, 0x02, 0x6f, 0x6b]));
  client.destroy();
});

test('upstream unavailable yields a finite failure', async t => {
  const unused = net.createServer(); unused.listen(0, '127.0.0.1'); await once(unused, 'listening');
  const upstreamPort = unused.address().port; await new Promise(r => unused.close(r));
  const probe = createProbe(new Map([['probe', upstreamPort]]));
  t.after(() => probe.close());
  probe.server.listen(0, '127.0.0.1'); await once(probe.server, 'listening');
  assert.equal((await request(probe.server.address().port)).code, 502);
});

test('duplicate Host is rejected without contacting any upstream', async t => {
  let hits = 0;
  const {port} = await fixture(t, (_, res) => { hits++; res.end(); });
  const client = net.connect(port, '127.0.0.1');
  t.after(() => client.destroy()); await once(client, 'connect');
  const response = once(client, 'data');
  client.write(`GET / HTTP/1.1\r\nHost: probe.localhost:${port}\r\nHost: attacker.example\r\n\r\n`);
  assert.match((await response)[0].toString(), /^HTTP\/1.1 421 /);
  assert.equal(hits, 0);
});

test('chunked POST body is forwarded without changing payload bytes', async t => {
  const payload = Buffer.alloc(1024 * 1024, 0xa5);
  const digest = createHash('sha256').update(payload).digest('hex');
  const {port} = await fixture(t, (req, res) => {
    const hash = createHash('sha256');
    assert.equal(req.method, 'POST');
    req.on('data', chunk => hash.update(chunk));
    req.on('end', () => res.end(hash.digest('hex')));
  });
  const result = await new Promise((resolve, reject) => {
    const req = http.request({hostname: '127.0.0.1', port, method: 'POST',
      headers: {host: `probe.localhost:${port}`, 'transfer-encoding': 'chunked'}}, res => {
      let body = ''; res.on('data', chunk => body += chunk); res.on('end', () => resolve(body));
    });
    req.on('error', reject); req.write(payload.subarray(0, 333)); req.end(payload.subarray(333));
  });
  assert.equal(result, digest);
});

test('rejected WebSocket upgrade returns the upstream status and closes', async t => {
  const {port} = await fixture(t, (_, res) => { res.writeHead(403); res.end('blocked'); });
  const client = net.connect(port, '127.0.0.1');
  t.after(() => client.destroy()); await once(client, 'connect');
  const response = once(client, 'data');
  client.write(`GET /ws HTTP/1.1\r\nHost: probe.localhost:${port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n`);
  assert.match((await response)[0].toString(), /^HTTP\/1.1 403 /);
  await once(client, 'end');
});
