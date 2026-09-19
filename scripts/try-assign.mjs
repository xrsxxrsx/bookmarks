// Exercises the manual merge over the window's own IPC, so the real path is tested:
// plan → reassign to an existing work → apply.
//
// Usage: node scripts/try-assign.mjs <port> <probe-file> <target-work-id>

const [portArg, probeFile, targetArg] = process.argv.slice(2);
if (!portArg || !probeFile || !targetArg) {
  console.error('usage: try-assign.mjs <port> <probe-file> <target-work-id>');
  process.exit(2);
}
const PORT = Number(portArg);
const targetWorkId = Number(targetArg);

const list = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = list.find((t) => t.type === 'page');
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

async function evaluate(expression) {
  const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) {
    throw new Error(r.exceptionDetails.exception?.description ?? JSON.stringify(r.exceptionDetails));
  }
  return r.result.value;
}

/** Calls a Tauri command through the page's IPC bridge. */
const invoke = (cmd, args) =>
  evaluate(`(async () => {
    try {
      return await window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)});
    } catch (e) {
      return { __error: String(e && e.message ? e.message : e) };
    }
  })()`);

function fail(label, result) {
  if (result && result.__error) {
    console.error(`${label} failed: ${result.__error}`);
    process.exit(1);
  }
  return result;
}

const describe = (preview) =>
  preview.works
    .map(
      (w) =>
        `  key=${w.key} -> ${w.is_new ? 'NEW' : `#${w.existing_work_id}`}` +
        ` manual=${w.manual_target ?? '-'} files=${w.files.length}`,
    )
    .join('\n');

// ---------------------------------------------------------------- step 1: plan

const preview = fail('analyze_import', await invoke('analyze_import', { paths: [probeFile] }));
console.log('=== plan (automatic) ===');
console.log(describe(preview));
console.log(`  candidates offered: ${preview.candidates.length}`);
console.log(`  keep both versions: ${preview.keep_both_versions}`);
const target = preview.candidates.find((c) => c.id === targetWorkId);
if (target) {
  console.log(`  target #${target.id} "${target.title}" formats=[${target.formats.join(', ')}]`);
} else {
  console.error(`  target #${targetWorkId} is not among the candidates`);
  process.exit(1);
}

const key = preview.works[0].key;
if (preview.works[0].is_new !== true) {
  console.error('the probe file was expected to plan as a new work');
  process.exit(1);
}

// ---------------------------------------------------------------- step 2: reassign

const assigned = fail(
  'assign_import_target',
  await invoke('assign_import_target', { token: preview.token, key, workId: targetWorkId }),
);
console.log('\n=== plan (after choosing the target) ===');
console.log(describe(assigned));
const work = assigned.works[0];
if (work.existing_work_id !== targetWorkId) {
  console.error(`expected to merge into #${targetWorkId}, got ${work.existing_work_id}`);
  process.exit(1);
}
if (work.manual_target !== targetWorkId) {
  console.error('the choice was not recorded as manual');
  process.exit(1);
}
console.log(`  summary: new=${assigned.summary.works_new} existing=${assigned.summary.works_existing}`);
console.log(`  version choice needed: ${work.files[0].needs_version_choice}`);

// ---------------------------------------------------------------- step 3: withdraw

const reverted = fail(
  'assign_import_target (withdraw)',
  await invoke('assign_import_target', { token: assigned.token, key, workId: null }),
);
console.log('\n=== plan (after withdrawing the choice) ===');
console.log(describe(reverted));
if (reverted.works[0].is_new !== true) {
  console.error('withdrawing the choice should return it to a new work');
  process.exit(1);
}

// ---------------------------------------------------------------- step 4: apply

const reapplied = fail(
  'assign_import_target (re-apply)',
  await invoke('assign_import_target', { token: reverted.token, key, workId: targetWorkId }),
);
const report = fail(
  'execute_import',
  await invoke('execute_import', { token: reapplied.token, keepBoth: null }),
);
console.log('\n=== applied ===');
console.log(`  works created : ${JSON.stringify(report.works_created)}`);
console.log(`  works matched : ${JSON.stringify(report.works_matched)}`);
console.log(`  files stored  : ${report.files_stored}`);
for (const p of report.stored_paths ?? []) console.log(`    library/${p}`);

const library = fail('load_library', await invoke('load_library', {}));
const merged = library.works.find((w) => w.id === targetWorkId);
console.log(`\n  #${merged.id} "${merged.title}" now holds ${merged.files.length} file(s):`);
for (const f of merged.files) {
  console.log(`    ${f.format_class.padEnd(6)} ${f.role.padEnd(9)} v${f.version}  ${f.rel_path}`);
}

ws.close();
