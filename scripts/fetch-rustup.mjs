// Downloads rustup-init.exe through the local HTTP proxy.
//
// Why this exists: in this environment Schannel (used by PowerShell/curl/rustup's
// own TLS) cannot acquire credentials, so every https request from those tools fails
// with SEC_E_NO_CREDENTIALS. Node ships its own TLS stack and works fine, so we use it
// as the transport for bootstrapping.
//
// Usage: node scripts/fetch-rustup.mjs
import { createWriteStream } from 'node:fs';
import { mkdir, stat } from 'node:fs/promises';
import { get } from 'node:https';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const PROXY = process.env.DSH_PROXY ?? 'http://127.0.0.1:23333';
const URL_ =
  'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe';

const here = dirname(fileURLToPath(import.meta.url));
const dest = resolve(here, '..', '.tools', 'rustup-init.exe');

await mkdir(dirname(dest), { recursive: true });

/** Follow redirects manually so proxying stays predictable. */
function download(url, out, redirectsLeft = 5) {
  return new Promise((res, rej) => {
    const req = get(
      url,
      { timeout: 120_000, headers: { 'user-agent': 'bookmarks-bootstrap' } },
      (response) => {
        const { statusCode = 0, headers } = response;
        if (statusCode >= 300 && statusCode < 400 && headers.location) {
          response.resume();
          if (redirectsLeft <= 0) return rej(new Error('too many redirects'));
          return download(new URL(headers.location, url).href, out, redirectsLeft - 1)
            .then(res, rej);
        }
        if (statusCode !== 200) {
          response.resume();
          return rej(new Error(`HTTP ${statusCode} for ${url}`));
        }
        const expected = Number(headers['content-length'] ?? 0);
        const file = createWriteStream(out);
        let seen = 0;
        response.on('data', (chunk) => { seen += chunk.length; });
        response.pipe(file);
        file.on('finish', () => file.close(() => res({ seen, expected })));
        file.on('error', rej);
        response.on('error', rej);
      },
    );
    req.on('timeout', () => { req.destroy(new Error('request timed out')); });
    req.on('error', rej);
  });
}

const { seen, expected } = await download(URL_, dest);
const { size } = await stat(dest);
if (expected && size !== expected) {
  throw new Error(`size mismatch: got ${size}, expected ${expected}`);
}
console.log(`downloaded ${dest} (${size} bytes, received ${seen})`);
