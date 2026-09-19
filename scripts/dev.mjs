// Starts the application for development in the right order, and keeps it running.
//
// Needed because a debug build of the desktop shell loads the frontend from the dev server
// rather than from bundled files, so the server must already be listening. Launching the
// executable alone gives a "connection refused" page — which is exactly what it is, not a bug in
// the app. This script checks, starts what is missing, and then reports where the window is.
//
// Usage: node scripts/dev.mjs [--port 9223]

import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const DEV_URL = 'http://localhost:5173/';
const EXE = resolve(root, 'target', 'debug', 'bookmarks-app.exe');

const debugPortIndex = process.argv.indexOf('--port');
const DEBUG_PORT = debugPortIndex === -1 ? 9223 : Number(process.argv[debugPortIndex + 1]);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Is something answering on the dev server? */
async function devServerUp() {
  try {
    const res = await fetch(DEV_URL, { signal: AbortSignal.timeout(2000) });
    return res.ok;
  } catch {
    return false;
  }
}

/** Waits for a condition, so nothing races a fixed delay. */
async function waitFor(label, check, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await check()) return true;
    await sleep(400);
  }
  console.error(`timed out waiting for ${label}`);
  return false;
}

if (!existsSync(EXE)) {
  console.error(`the application is not built: ${EXE}`);
  console.error('build it first:  cargo build --workspace');
  process.exit(1);
}

// Vite must already be listening: the debug build reads the frontend from it.
if (!(await devServerUp())) {
  console.log('starting the dev server…');
  const vite = spawn('cmd.exe', ['/c', 'npm run dev'], {
    cwd: root,
    stdio: 'ignore',
    detached: true,
    windowsHide: true,
  });
  vite.unref();
  if (!(await waitFor('the dev server', devServerUp, 60_000))) {
    console.error('the dev server did not come up; run `npm run dev` in a terminal to see why');
    process.exit(1);
  }
}
console.log(`dev server ok    : ${DEV_URL}`);

// The debugging port lets the verification scripts inspect the live window.
const already = await fetch(`http://127.0.0.1:${DEBUG_PORT}/json`).then(
  () => true,
  () => false,
);

const child = spawn(EXE, [], {
  cwd: root,
  detached: true,
  stdio: 'ignore',
  windowsHide: true,
  env: already
    ? process.env
    : { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${DEBUG_PORT}` },
});
child.unref();

await sleep(1500);
console.log(`application      : pid ${child.pid}`);
console.log(
  already
    ? `debug port ${DEBUG_PORT}  : already in use, so the running window keeps it`
    : `debug port ${DEBUG_PORT}  : enabled for scripts/verify-ui.mjs`,
);
console.log('\nThe window should be open. If it shows "connection refused", the dev server died —');
console.log('re-run this script, which will start it again.');
