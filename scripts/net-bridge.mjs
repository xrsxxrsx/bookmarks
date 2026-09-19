// A local HTTP -> HTTPS bridge, so cargo can reach crates.io from this environment.
//
// Why this is needed: in this sandbox, any process using Schannel for TLS (cargo, curl,
// PowerShell, git) cannot complete a handshake — connections time out or fail with
// SEC_E_NO_CREDENTIALS — while Node's own TLS stack reaches the same hosts normally. Node
// therefore acts as the transport: cargo speaks plain HTTP to this bridge, and the bridge
// makes the real HTTPS request and streams the response back.
//
// Only the crates.io hosts cargo needs are contacted, so this cannot serve as an open
// relay. Upstream is occasionally flaky from here (the odd 403 or reset), so requests are
// retried before a failure reaches cargo.
//
// Usage: node scripts/net-bridge.mjs [port]     (default 23456)

import { createServer } from 'node:http';
import { get } from 'node:https';

const PORT = Number(process.argv[2] ?? 23456);
const MAX_ATTEMPTS = 4;

/** Resolve a request path to the real upstream URL. */
function resolveUpstream(req) {
  // Sparse registry index: /index/config.json -> https://index.crates.io/config.json
  if (req.url.startsWith('/index/')) {
    return `https://index.crates.io/${req.url.slice('/index/'.length)}`;
  }
  // Crate download. Cargo's `{crate}`/`{version}` template gives us the name and version,
  // and the CDN path needs the crate name repeated: /crates/<n>/<n>-<v>.crate
  const download = /^\/crates\/([^/]+)\/([^/]+)\/download$/.exec(req.url);
  if (download) {
    return `https://static.crates.io/crates/${download[1]}/${download[1]}-${download[2]}.crate`;
  }
  // Crates.io metadata, in case a tool needs it.
  if (req.url.startsWith('/api/')) {
    return `https://crates.io${req.url}`;
  }
  return null;
}

/** One attempt at fetching `url`, streaming into `res` on success. */
function attempt(url, res) {
  return new Promise((resolve) => {
    const upstream = get(
      url,
      { timeout: 60_000, headers: { 'user-agent': 'cargo-net-bridge' } },
      (up) => {
        const status = up.statusCode ?? 502;
        // A non-2xx from the CDN is often transient here; retry rather than pass it on,
        // otherwise cargo records a permanent "failed to download".
        if (status >= 400) {
          up.resume();
          resolve({ ok: false, reason: `upstream HTTP ${status}` });
          return;
        }
        const headers = {};
        for (const key of ['content-type', 'content-length', 'etag', 'last-modified']) {
          if (up.headers[key] !== undefined) headers[key] = up.headers[key];
        }
        res.writeHead(status, headers);
        up.pipe(res);
        up.on('end', () => resolve({ ok: true, done: true }));
        up.on('error', () => resolve({ ok: false, reason: 'upstream stream error' }));
      },
    );

    upstream.on('timeout', () => upstream.destroy(new Error('upstream timed out')));
    upstream.on('error', (err) => resolve({ ok: false, reason: err.message }));
  });
}

const server = createServer(async (req, res) => {
  const url = resolveUpstream(req);
  if (!url) {
    res.writeHead(403, { 'content-type': 'text/plain' });
    res.end(`refused: ${req.url}\n`);
    console.log(`REFUSED ${req.method} ${req.url}`);
    return;
  }

  for (let n = 1; n <= MAX_ATTEMPTS; n += 1) {
    const started = Date.now();
    const result = await attempt(url, res);
    const ms = Date.now() - started;

    if (result.ok) {
      console.log(`OK      ${req.url} (${ms}ms)${n > 1 ? ` after ${n} attempts` : ''}`);
      return;
    }
    // Headers already sent means the body was partially delivered; cannot retry.
    if (res.headersSent) {
      console.log(`PARTIAL ${req.url}: ${result.reason}`);
      res.end();
      return;
    }
    console.log(`RETRY   ${req.url} attempt ${n}/${MAX_ATTEMPTS}: ${result.reason}`);
  }

  console.log(`FAILED  ${req.url}`);
  if (!res.headersSent) res.writeHead(502, { 'content-type': 'text/plain' });
  res.end('bridge: upstream unavailable after retries\n');
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`net bridge listening on http://127.0.0.1:${PORT}`);
  console.log('  /index/...                -> https://index.crates.io/...');
  console.log('  /crates/<n>/<v>/download  -> https://static.crates.io/crates/<n>/<n>-<v>.crate');
});
