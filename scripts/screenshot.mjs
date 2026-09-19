// Captures a screenshot of the running window over the Chrome DevTools Protocol.
//
// Usage: node scripts/screenshot.mjs [port] [outfile]
//
// Kept as a script rather than a one-off because the READMEs reference a screenshot, and a
// stale screenshot is worse than none: re-running this is how it gets refreshed after a
// change to the interface.

import { writeFileSync } from 'node:fs';

const PORT = Number(process.argv[2] ?? 9223);
const OUT = process.argv[3] ?? 'docs/screenshot.png';

const list = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = list.find((t) => t.type === 'page');
if (!page) throw new Error('no page target');

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => {
  ws.addEventListener('open', res);
  ws.addEventListener('error', rej);
});

let id = 1;
const call = (method, params) => {
  return new Promise((res, rej) => {
    const myId = id++;
    const onMsg = (e) => {
      const m = JSON.parse(e.data);
      if (m.id === myId) {
        ws.removeEventListener('message', onMsg);
        m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result);
      }
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id: myId, method, params }));
  });
};

const evaluate = async (expression) => {
  const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? 'probe threw');
  return r.result.value;
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Wait for the library to have rendered rather than guessing at a delay: an empty table in
// the screenshot would misrepresent the application.
for (let attempt = 0; attempt < 40; attempt += 1) {
  const rows = await evaluate(`document.querySelectorAll('tbody tr').length`);
  if (rows > 0) {
    console.log(`table rendered: ${rows} rows`);
    break;
  }
  await sleep(250);
}

// Select the first work so the detail panel is populated — the screenshot should show what
// the application actually looks like in use, not an empty selection state.
const selected = await evaluate(`(async () => {
  const row = document.querySelector('tbody tr');
  if (!row) return false;
  row.click();
  await new Promise(r => setTimeout(r, 700));
  return !!document.querySelector('.detail-actions button');
})()`);
console.log(`detail panel populated: ${selected}`);

await sleep(600);

const shot = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
writeFileSync(OUT, Buffer.from(shot.data, 'base64'));

const size = await evaluate(`({ w: window.innerWidth, h: window.innerHeight })`);
console.log(`wrote ${OUT} at ${size.w}x${size.h}`);
ws.close();
