// Inspects the running application's live DOM over the Chrome DevTools Protocol.
//
// Why this exists: launching the app and seeing that the process is alive proves nothing
// about what the window shows. WebView2 exposes CDP when started with a remote debugging
// port, so the rendered result can actually be read back — which is the only way to check
// the interface without a human looking at it.
//
// Usage: node scripts/inspect-ui.mjs [port]

const PORT = Number(process.argv[2] ?? 9223);

async function targets() {
  const res = await fetch(`http://127.0.0.1:${PORT}/json`);
  return res.json();
}

async function waitForPage(timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = 'no targets yet';
  while (Date.now() < deadline) {
    try {
      const list = await targets();
      const page = list.find((t) => t.type === 'page' && t.webSocketDebuggerUrl);
      if (page) return page;
      lastError = `targets present but no page: ${list.map((t) => t.type).join(',')}`;
    } catch (e) {
      lastError = e.message;
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`no debuggable page after ${timeoutMs}ms: ${lastError}`);
}

/** Minimal CDP client over the built-in WebSocket. */
function connect(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    let nextId = 1;
    const pending = new Map();

    ws.addEventListener('open', () => resolve({ send, close: () => ws.close() }));
    ws.addEventListener('error', (e) => reject(new Error(`websocket error: ${e.message ?? 'unknown'}`)));
    ws.addEventListener('message', (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id !== undefined && pending.has(msg.id)) {
        const { resolve: res, reject: rej } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.error) rej(new Error(JSON.stringify(msg.error)));
        else res(msg.result);
      }
    });

    function send(method, params = {}) {
      const id = nextId++;
      return new Promise((res, rej) => {
        pending.set(id, { resolve: res, reject: rej });
        ws.send(JSON.stringify({ id, method, params }));
      });
    }
  });
}

/** The probe, evaluated inside the page. Kept as one function so it can return a report. */
const PROBE = `(() => {
  const text = (el) => (el ? el.textContent.trim().replace(/\\s+/g, ' ') : null);
  const rows = [...document.querySelectorAll('tbody tr')];
  const describe = (row) => {
    const cell = (sel) => text(row.querySelector(sel));
    return {
      title: cell('.col-title'),
      author: cell('.col-author'),
      lang: cell('.col-lang'),
      words: cell('.col-words'),
      date: cell('.col-date'),
      tags: cell('.col-tags'),
      classes: row.className,
    };
  };
  const doc = document.documentElement;
  return {
    title: document.title,
    rootChildren: document.getElementById('root')?.children.length ?? -1,
    topbar: text(document.querySelector('.topbar .brand')),
    fatal: text(document.querySelector('.fatal')),
    loading: text(document.querySelector('.loading')),
    banners: [...document.querySelectorAll('.banner')].map(text),
    headers: [...document.querySelectorAll('thead th')].map(text),
    rowCount: rows.length,
    rows: rows.map(describe),
    nestedRows: document.querySelectorAll('tbody tr.nested').length,
    translationChips: document.querySelectorAll('.chip.translation').length,
    searchPlaceholder: document.querySelector('.filterbar .search')?.placeholder ?? null,
    sortSelect: text(document.querySelector('.sortgroup select')),
    buttons: [...document.querySelectorAll('.topbar-actions button')].map(text),
    statusbar: text(document.querySelector('.statusbar')),
    detailTitle: text(document.querySelector('.detail h2')),
    detailSections: [...document.querySelectorAll('.detail h3')].map(text),
    scrollOverflowX: doc.scrollWidth > doc.clientWidth,
    scrollWidth: doc.scrollWidth,
    clientWidth: doc.clientWidth,
    tableWidth: document.querySelector('table')?.getBoundingClientRect().width ?? null,
    detailWidth: document.querySelector('.detail')?.getBoundingClientRect().width ?? null,
  };
})()`;

async function main() {
  const page = await waitForPage();
  console.log(`target: ${page.url}`);

  const client = await connect(page.webSocketDebuggerUrl);
  // Give the React tree a moment to mount and fetch the library over IPC.
  await new Promise((r) => setTimeout(r, 2500));

  const result = await client.send('Runtime.evaluate', {
    expression: PROBE,
    returnByValue: true,
    awaitPromise: false,
  });

  if (result.exceptionDetails) {
    console.error('probe threw:', JSON.stringify(result.exceptionDetails, null, 2));
    process.exit(1);
  }

  const report = result.result.value;
  console.log('\n===== RENDERED INTERFACE =====');
  console.log(`document.title        : ${report.title}`);
  console.log(`root children         : ${report.rootChildren}`);
  console.log(`fatal panel           : ${report.fatal ?? '(none)'}`);
  console.log(`loading panel         : ${report.loading ?? '(none)'}`);
  console.log(`brand                 : ${report.topbar}`);
  console.log(`search placeholder    : ${report.searchPlaceholder}`);
  console.log(`sort control          : ${report.sortSelect}`);
  console.log(`toolbar buttons       : ${report.buttons.join(' | ')}`);
  console.log(`table headers         : ${report.headers.join(' | ')}`);
  console.log(`rows rendered         : ${report.rowCount}  (nested: ${report.nestedRows}, translation chips: ${report.translationChips})`);
  console.log(`status bar            : ${report.statusbar}`);
  console.log(`detail panel title    : ${report.detailTitle ?? '(empty state)'}`);
  console.log(`detail sections       : ${report.detailSections.join(' | ')}`);
  console.log(`layout: doc ${report.scrollWidth}px / client ${report.clientWidth}px -> horizontal overflow: ${report.scrollOverflowX}`);
  console.log(`table ${report.tableWidth}px, detail ${report.detailWidth}px`);

  if (report.banners.length > 0) {
    console.log('\nbanners:');
    for (const b of report.banners) console.log(`  - ${b}`);
  }

  console.log('\nrows:');
  for (const r of report.rows) {
    console.log(`  [${r.classes}]`);
    console.log(`    title : ${r.title}`);
    console.log(`    author: ${r.author} | lang: ${r.lang} | words: ${r.words} | date: ${r.date}`);
    console.log(`    tags  : ${r.tags}`);
  }
  console.log('===== END =====');

  client.close();
}

main().catch((e) => {
  console.error('FAILED:', e.message);
  process.exit(1);
});
