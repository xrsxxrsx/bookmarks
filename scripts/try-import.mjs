// Drives the running application's import pipeline over the Chrome DevTools Protocol, so the
// real path is exercised: the window's own IPC bridge, the plan, and the files landing on disk.
//
// Usage: node scripts/try-import.mjs <port> <file> [file...] [--apply]

const [portArg, ...rest] = process.argv.slice(2);
const PORT = Number(portArg ?? 9223);
const apply = rest.includes('--apply');
const files = rest.filter((a) => a !== '--apply');

if (files.length === 0) {
  console.error('usage: try-import.mjs <port> <file...> [--apply]');
  process.exit(2);
}

const list = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = list.find((t) => t.type === 'page');
if (!page) throw new Error('no page target');

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => {
  ws.addEventListener('open', res);
  ws.addEventListener('error', rej);
});
let id = 1;
const call = (method, params) =>
  new Promise((res, rej) => {
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

/** Runs an expression in the page and returns its value, surfacing page-side errors. */
async function evaluate(expression) {
  const r = await call('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (r.exceptionDetails) {
    throw new Error(
      `page error: ${r.exceptionDetails.exception?.description ?? JSON.stringify(r.exceptionDetails)}`,
    );
  }
  return r.result.value;
}

const paths = JSON.stringify(files);

// The window exposes no globals, so the import is driven through the same IPC the UI uses.
const preview = await evaluate(`(async () => {
  const { invoke } = window.__TAURI_INTERNALS__ ? window.__TAURI_INTERNALS__ : {};
  if (!invoke) return { error: 'no IPC bridge on window' };
  try {
    return await invoke('analyze_import', { paths: ${paths} });
  } catch (e) {
    return { error: String(e && e.message ? e.message : e) };
  }
})()`);

if (preview.error) {
  console.error('analyze_import failed:', preview.error);
  process.exit(1);
}

console.log('=== plan ===');
console.log(
  `  ${preview.summary.files_total} file(s): ${preview.summary.files_to_store} to store, ` +
    `${preview.summary.duplicates} duplicate(s)`,
);
console.log(
  `  ${preview.summary.works_new} new work(s), ${preview.summary.works_existing} to merge`,
);
console.log(`  default: keep both versions = ${preview.keep_both_versions}`);

for (const work of preview.works) {
  console.log(`\n  ${work.is_new ? 'NEW ' : `#${work.existing_work_id}`} ${work.title}`);
  for (const file of work.files) {
    console.log(
      `      ${file.format_class.padEnd(6)} (${file.format_detail.padEnd(4)}) ` +
        `${file.original_name.padEnd(34)} ${String(file.action.kind).padEnd(20)} ` +
        `versionChoice=${file.needs_version_choice}` +
        (file.word_count === null ? '' : ` words=${file.word_count_is_estimated ? '~' : ''}${file.word_count}`),
    );
  }
}

if (!apply) {
  console.log('\n(dry run — pass --apply to execute)');
  ws.close();
  process.exit(0);
}

const report = await evaluate(`(async () => {
  const { invoke } = window.__TAURI_INTERNALS__;
  try {
    return await invoke('execute_import', { token: ${JSON.stringify(preview.token)}, keepBoth: null });
  } catch (e) {
    return { error: String(e && e.message ? e.message : e) };
  }
})()`);

if (report.error) {
  console.error('execute_import failed:', report.error);
  process.exit(1);
}

console.log('\n=== result ===');
console.log(`  works created : ${JSON.stringify(report.works_created)}`);
console.log(`  works matched : ${JSON.stringify(report.works_matched)}`);
console.log(`  files stored  : ${report.files_stored}`);
console.log(`  files skipped : ${report.files_skipped}`);
console.log(`  pairs linked  : ${report.relations_stored}`);
for (const p of report.stored_paths ?? []) console.log(`    library/${p}`);
if (report.errors?.length) {
  console.log('  errors:');
  for (const [path, err] of report.errors) console.log(`    ${path}: ${err}`);
}

ws.close();
