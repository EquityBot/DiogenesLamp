import {spawn} from 'node:child_process';
import {mkdtemp, mkdir, readFile, writeFile, rm, readdir as fsDirectory} from 'node:fs/promises';
import {join, resolve, dirname} from 'node:path';
import {tmpdir} from 'node:os';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
import {createServer} from 'node:net';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const args = process.argv.slice(2);
const benchmark = args.includes('--benchmark');
if (process.platform === 'win32') throw new Error('This driver requires POSIX process groups for owned-tree cleanup; Windows needs an isolated job runner');
const option = name => { const i = args.indexOf(name); if (i < 0) return undefined; if (!args[i + 1] || args[i + 1].startsWith('--')) throw new Error('Missing value: ' + name); return args[i + 1]; };
const allowed = new Set(['--benchmark', '--lamp', '--generator', '--browser', '--output', '--runs']);
for (let i = 0; i < args.length; i++) { if (!allowed.has(args[i])) throw new Error('Unknown option: ' + args[i]); if (args[i] !== '--benchmark') i++; }
const repeats = Number(option('--runs') ?? (benchmark ? 5 : 1));
if (!Number.isInteger(repeats) || repeats < 1 || repeats > 10) throw new Error('--runs must be an integer from 1 to 10');
const extension = process.platform === 'win32' ? '.exe' : '';
const target = resolve(repo, process.env.CARGO_TARGET_DIR || 'target', 'release');
const lamp = resolve(option('--lamp') || join(target, 'lamp' + extension));
const generator = resolve(option('--generator') || join(target, 'examples', 'make_demo_data' + extension));
const browser = option('--browser') || process.env.BROWSER_BIN;
if (!browser) throw new Error('Set BROWSER_BIN or pass --browser with an installed Chromium-compatible executable');
const run = await mkdtemp(join(tmpdir(), 'lamp-browser-'));
const childEnv = Object.fromEntries(['PATH', 'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT', 'LANG', 'LC_ALL', 'LD_LIBRARY_PATH'].filter(k => process.env[k]).map(k => [k, process.env[k]]));
Object.assign(childEnv, {HOME: run, TMPDIR: run, TEMP: run, TMP: run, LOCALAPPDATA: run, APPDATA: run, PYTHONUNBUFFERED: '1'});
const profile = await mkdtemp(join(run, 'browser-profile-'));
const downloads = join(run, 'browser-downloads');
await mkdir(downloads, {recursive: true});
const children = [];
const result = {scope: 'Fabricated release-build catalogue, 100000 raw records; fresh contexts and warm reloads, not OS-cache-cold or mobile hardware', repeats, runs: [], memory: {scope: 'Linux sampled aggregate RSS of owned child process groups; shared pages counted per process; not physical memory or an unsampled peak', interval_ms: 250, samples: 0, peak_bytes: null}};
let sampling = false;
const memoryTimer = process.platform === 'linux' ? setInterval(async () => {
  if (sampling) return;
  sampling = true;
  try {
    const groups = new Set(children.map(c => c.pid));
    let total = 0;
    for (const pid of await fsDirectory('/proc')) {
      if (!/^\d+$/.test(pid)) continue;
      try {
        const stat = await readFile('/proc/' + pid + '/stat', 'utf8');
        const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
        if (!groups.has(Number(fields[2]))) continue;
        const status = await readFile('/proc/' + pid + '/status', 'utf8');
        total += Number(status.match(/^VmRSS:\s+(\d+) kB/m)?.[1] || 0) * 1024;
      } catch { /* A child may exit between the metadata reads. */ }
    }
    result.memory.samples++;
    result.memory.peak_bytes = Math.max(result.memory.peak_bytes || 0, total);
  } catch (error) { result.memory.error = String(error); }
  finally { sampling = false; }
}, 250) : undefined;
let socket;
let failed = false;
const deadline = setTimeout(() => { failed = true; result.status = 'fail'; result.error = 'Overall deadline exceeded'; cleanup().finally(() => { console.log(JSON.stringify(result)); process.exit(124); }); }, benchmark ? 600000 : 180000);

let cleanupTask;
function cleanup() {
  return cleanupTask ??= cleanupOwned();
}
async function cleanupOwned() {
  clearInterval(memoryTimer);
  if (socket?.readyState === WebSocket.OPEN) { try { await send('Browser.close'); } catch {} socket.close(); }
  for (const child of children) {
    if (child.ownedGroup) {
      // The leader may have exited while descendants still own the group.
      try { process.kill(-child.pid, 'SIGTERM'); } catch {}
      const until = Date.now() + 3000;
      while (Date.now() < until) {
        try { process.kill(-child.pid, 0); } catch { break; }
        await new Promise(resolve => setTimeout(resolve, 50));
      }
      try { process.kill(-child.pid, 'SIGKILL'); } catch {}
    } else if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGKILL');
      await new Promise(resolve => child.once('exit', resolve));
    }
  }
  await rm(run, {recursive: true, force: true, maxRetries: 10, retryDelay: 200});
}

function launch(executable, args, pattern) {
  const child = spawn(executable, args, {detached: process.platform !== 'win32', windowsHide: true, cwd: repo, env: childEnv, stdio: ['ignore', 'pipe', 'pipe']});
  child.ownedGroup = true;
  children.push(child);
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(new Error(`Child readiness timed out: ${executable}: ${output.slice(-2000)}`)), 15000);
    const capture = bytes => {
      output = (output + bytes.toString()).slice(-100000);
      const match = output.match(pattern);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    };
    child.stdout.on('data', capture); child.stderr.on('data', capture);
    child.once('error', reject);
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Child exited ${code}: ${output.slice(-1000)}`)); });
  });
}

let sequence = 0;
const pending = new Map();
const completedDownloads = new Set();
function send(method, params = {}, sessionId) {
  return new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 15000);
    pending.set(id, {resolve, reject, timer});
    socket.send(JSON.stringify({id, method, params, ...(sessionId ? {sessionId} : {})}));
  });
}
async function evaluate(session, expression) {
  const response = await send('Runtime.evaluate', {expression, returnByValue: true, awaitPromise: true}, session);
  if (response.exceptionDetails) throw new Error(JSON.stringify(response.exceptionDetails));
  return response.result.value;
}
async function waitFor(check, label, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const value = await check(); if (value) return value; await new Promise(resolve => setTimeout(resolve, 40)); }
  throw new Error(`Timed out: ${label}`);
}
const frames = session => evaluate(session, 'new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
async function key(session, value) {
  const vk = {Tab: 9, Enter: 13, ArrowRight: 39, End: 35, Home: 36, p: 80}[value];
  const physical = {key: value, code: value === 'p' ? 'KeyP' : value, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk};
  await send('Input.dispatchKeyEvent', {type: 'keyDown', ...physical}, session);
  await send('Input.dispatchKeyEvent', {type: 'keyUp', ...physical}, session);
  await frames(session);
}
async function focusState(session) {
  return evaluate(session, `(() => {const cards=[...document.querySelectorAll('#grid > .card[tabindex="0"]')]; const a=document.activeElement; return {stops:cards.length,connected:a?.isConnected,activeCard:a===cards[0],pos:Number(a?.getAttribute('aria-posinset')),size:Number(a?.getAttribute('aria-setsize')),count:state.filtered.length};})()`);
}
async function upload(session, path) {
  await evaluate(session, 'document.querySelector("#catalogue-tools").open=true;document.querySelector("#action-status").textContent=""');
  const {root} = await send('DOM.getDocument', {}, session);
  const {nodeId} = await send('DOM.querySelector', {nodeId: root.nodeId, selector: '#import-backup'}, session);
  await send('DOM.setFileInputFiles', {nodeId, files: [path]}, session);
  await waitFor(() => evaluate(session, 'document.querySelector("#action-status").textContent.startsWith("Imported ")'), 'import');
  return evaluate(session, 'document.querySelector("#action-status").textContent');
}
async function download(session) {
  const previous = new Set(completedDownloads);
  await evaluate(session, 'document.querySelector("#export-favs").click()');
  const guid = await waitFor(() => [...completedDownloads].find(id => !previous.has(id)), 'download');
  const path = join(downloads, guid);
  return {path, backup: JSON.parse(await readFile(path, 'utf8'))};
}
const ordered = entries => [...entries].sort((a, b) => (a.kind + a.id).localeCompare(b.kind + b.id));

try {
  await new Promise((resolveDone, reject) => {
    const child = spawn(generator, [join(run, 'data'), '10', '10000', '100'], {cwd: repo, env: childEnv, windowsHide: true, stdio: ['ignore', 'ignore', 'pipe']});
    children.push(child); let diagnostic = '';
    child.stderr.on('data', b => { diagnostic = (diagnostic + b).slice(-4000); });
    child.once('error', reject);
    child.once('exit', code => code === 0 ? resolveDone() : reject(new Error('Fixture generator exit ' + code + ': ' + diagnostic)));
  });
  const reservation = createServer();
  await new Promise((resolve, reject) => { reservation.once('error', reject); reservation.listen(0, '127.0.0.1', resolve); });
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  // The CLI requires a nonzero port. A bind race fails this run rather than reusing a server.
  const origin = await launch(lamp, ['--data', join(run, 'data'), 'serve', '--port', String(port), '--no-browser'], /serving on (http:\/\/127\.0\.0\.1:\d+\/)/i);
  const endpoint = await launch(browser, ['--headless=new', '--disable-dev-shm-usage', '--disable-background-networking', '--disable-component-update', '--disable-sync', '--disable-extensions', '--no-first-run', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'], /DevTools listening on (ws:\/\/\S+)/);
  socket = new WebSocket(endpoint);
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, {once: true}); socket.addEventListener('error', reject, {once: true}); });
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.id) {
      const item = pending.get(message.id); if (!item) return;
      clearTimeout(item.timer); pending.delete(message.id);
      if (message.error) item.reject(new Error(JSON.stringify(message.error))); else item.resolve(message.result);
    } else if (message.method === 'Browser.downloadProgress' && message.params.state === 'completed') completedDownloads.add(message.params.guid);
    else if (message.method === 'Fetch.requestPaused') {
      const allowed = message.params.request.url.startsWith(origin);
      send(allowed ? 'Fetch.continueRequest' : 'Fetch.failRequest', {requestId: message.params.requestId, ...(allowed ? {} : {errorReason: 'BlockedByClient'})}, message.sessionId).catch(() => {});
    }
  });
  result.browser = await send('Browser.getVersion');
  for (let repetition = 1; repetition <= repeats; repetition++) {
  let previousExport;
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    const {browserContextId} = await send('Target.createBrowserContext');
    await send('Browser.setDownloadBehavior', {behavior: 'allowAndName', downloadPath: downloads, eventsEnabled: true, browserContextId});
    const {targetId} = await send('Target.createTarget', {url: 'about:blank', browserContextId});
    const {sessionId: session} = await send('Target.attachToTarget', {targetId, flatten: true});
    for (const domain of ['Page', 'Runtime', 'Performance']) await send(`${domain}.enable`, {}, session);
    await send('Fetch.enable', {patterns: [{urlPattern: '*', requestStage: 'Request'}]}, session);
    await send('Emulation.setDeviceMetricsOverride', {width, height, deviceScaleFactor: 1, mobile: false}, session);
    await send('Page.addScriptToEvaluateOnNewDocument', {source: `window.measurement={longTasks:[],frames:[],firstCard:null};new PerformanceObserver(list=>measurement.longTasks.push(...list.getEntries().map(e=>e.duration))).observe({type:'longtask',buffered:true});let last;function frame(t){if(last&&measurement.frames.length<1000)measurement.frames.push(t-last);last=t;if(!measurement.firstCard&&document.querySelector('#grid > .card'))measurement.firstCard=performance.now();if(measurement.frames.length<1000||!measurement.firstCard)requestAnimationFrame(frame);}requestAnimationFrame(frame);`}, session);
    await send('Page.navigate', {url: origin}, session);
    await waitFor(() => evaluate(session, 'typeof state!=="undefined" && state.loadedSellers===10 && document.querySelector("#grid")?.getAttribute("aria-busy")==="false" && !!document.querySelector("#grid > .card")'), '100k catalogue load', 45000);
    const measurement = await evaluate(session, `({viewport:[innerWidth,innerHeight],complete_ms:performance.now(),raw:[...state.shardAlbums.values()].reduce((n,a)=>n+a.length,0),selected:state.albums.length,filtered:state.filtered.length,loadedSellers:state.loadedSellers,failedShards:state.failedShards,rendered:document.querySelectorAll('#grid > .card').length,first_card_ms:measurement.firstCard,long_tasks_ms:measurement.longTasks})`);
    assert.equal(measurement.raw, 100000); assert.equal(measurement.loadedSellers, 10); assert.deepEqual(measurement.failedShards, []);
    assert(Number.isFinite(measurement.first_card_ms), 'first card time was observed');
    await frames(session);
    measurement.initial_focus_state = await evaluate(session, `({stops:document.querySelectorAll('#grid > .card[tabindex="0"]').length,scrollY,cursor:state.view.cursor,first:[...document.querySelectorAll('#grid > .card')].slice(0,3).map(c=>({tab:c.getAttribute('tabindex'),pos:c.getAttribute('aria-posinset')}))})`);
    measurement.repetition = repetition;
    measurement.cache = 'fresh browser context; OS/server caches uncontrolled';
    result.runs.push(measurement);
    // A warm reload is measured independently before any saved-field edits.
    await evaluate(session, 'window.measurement=null');
    await send('Page.reload', {ignoreCache: false}, session);
    await waitFor(() => evaluate(session, 'window.measurement && typeof state!=="undefined" && state.loadedSellers===10 && document.querySelector("#grid")?.getAttribute("aria-busy")==="false" && !!document.querySelector("#grid > .card")'), 'warm catalogue load', 45000);
    measurement.warm_reload = await evaluate(session, '({complete_ms:performance.now(),first_card_ms:measurement.firstCard,long_tasks_ms:measurement.longTasks})');
    assert(Number.isFinite(measurement.warm_reload.first_card_ms), 'warm first card time was observed');
    await key(session, 'Tab');
    measurement.native_entry = [await evaluate(session, '({tag:document.activeElement.tagName,id:document.activeElement.id,href:document.activeElement.getAttribute("href"),scrollY})')];
    assert.equal(measurement.native_entry[0].href, '#grid');
    await key(session, 'Enter'); await key(session, 'Tab');
    const entered = await focusState(session); measurement.native_entry.push(entered);
    assert.equal(entered.activeCard, true);
    await key(session, 'ArrowRight');
    const advanced = await focusState(session); measurement.native_entry.push(advanced);
    assert.equal(advanced.pos, entered.pos + 1);
    await key(session, 'Home');
    if (previousExport) {
      assert.equal(await upload(session, previousExport.path), 'Imported 4 saved fields. Existing values were kept.');
      const immediate = await download(session);
      assert.deepEqual(ordered(immediate.backup.entries), ordered(previousExport.backup.entries));
      measurement.roundtrip_verified_before_edits = true;
    }
    // Set an explicit keyboard-test starting viewport; retain the initial anchoring observation above.
    await evaluate(session, 'window.scrollTo(0,0)'); await frames(session);
    await evaluate(session, 'document.querySelector(\'#grid > .card[tabindex="0"]\').focus()');
    const positions = [];
    for (const [pressed, expected] of [['ArrowRight', 2], ['End', measurement.filtered], ['Home', 1]]) {
      await key(session, pressed); const focus = await focusState(session);
      assert.equal(focus.stops, 1); assert.equal(focus.connected, true); assert.equal(focus.activeCard, true); assert.equal(focus.pos, expected); assert.equal(focus.size, measurement.filtered); positions.push({pressed, ...focus});
    }
    await evaluate(session, 'document.querySelector("#grid > .card .fav-btn").focus();window.scrollTo(0,document.documentElement.scrollHeight)');
    await frames(session);
    const rescued = await evaluate(session, '({connected:document.activeElement.isConnected,safe:document.activeElement.matches(".fav-btn, #grid")})');
    assert.equal(rescued.connected, true); assert.equal(rescued.safe, true);
    await evaluate(session, 'window.scrollTo(0,0)'); await frames(session);
    await evaluate(session, 'document.querySelector("#grid > .card").focus()'); await key(session, 'Home');
    await key(session, 'p');
    await waitFor(() => evaluate(session, 'document.querySelector("#album-preview").open && document.querySelector("#album-preview").contains(document.activeElement)'), 'preview focus');
    await evaluate(session, `document.querySelector('#preview-notes').value='Fabricated offline note';document.querySelector('#preview-tags').value='synthetic';document.querySelector('#preview-collection').value='Fixture';document.querySelector('#preview-form').requestSubmit()`);
    assert.equal(await evaluate(session, 'document.querySelector("#preview-save-status").textContent'), 'Saved.');
    await evaluate(session, 'document.querySelector("#preview-close").click()'); await frames(session);
    assert.equal((await focusState(session)).activeCard, true);
    const missing = join(run, `browser-missing-${width}.json`);
    await writeFile(missing, JSON.stringify({format: 'diogeneslamp', version: 1, entries: [{kind: 'note', id: '999999999999999', value: 'Fabricated unavailable listing'}]}));
    await upload(session, missing);
    const exported = await download(session);
    assert.equal(exported.backup.format, 'diogeneslamp'); assert.equal(exported.backup.version, 1); assert.equal(exported.backup.catalogue_status, 'complete'); assert(exported.backup.unresolved_ids.includes('999999999999999'));
    assert.equal(exported.backup.entries.length, 4);
    const savedIds = new Set(exported.backup.entries.filter(entry => entry.id !== '999999999999999').map(entry => entry.id));
    assert.equal(savedIds.size, 1, 'the 3 saved fields belong to one listing');
    const [savedId] = savedIds;
    assert.deepEqual(ordered(exported.backup.entries), ordered([
      {kind: 'note', id: '999999999999999', value: 'Fabricated unavailable listing'},
      {kind: 'collection', id: savedId, value: 'Fixture'},
      {kind: 'tags', id: savedId, value: ['synthetic']},
      {kind: 'note', id: savedId, value: 'Fabricated offline note'},
    ]));
    if (previousExport) assert.deepEqual(ordered(exported.backup.entries), ordered(previousExport.backup.entries));
    assert.equal(await upload(session, exported.path), 'Imported 0 saved fields. Existing values were kept.');
    const metrics = await send('Performance.getMetrics', {}, session);
    measurement.renderer_js_heap_used_bytes = metrics.metrics.find(metric => metric.name === 'JSHeapUsedSize')?.value;
    measurement.focus = positions; measurement.recycled_focus = rescued; measurement.saved_fields = exported.backup.entries; measurement.unresolved_ids = exported.backup.unresolved_ids;
    measurement.frame_intervals_ms = await evaluate(session, 'measurement.frames');
    previousExport = exported;
    if (benchmark && repetition === 1 && width === 390) {
      measurement.large_backups = [];
      for (const count of [1000, 10000]) {
        const entries = Array.from({length: count}, (_, i) => ({kind: 'note', id: String(800000000000000 + i), value: 'Fabricated saved note ' + i}));
        const file = join(run, 'backup-' + count + '.json');
        await writeFile(file, JSON.stringify({format: 'diogeneslamp', version: 1, entries}));
        const started = performance.now();
        const status = await upload(session, file);
        const importedMs = performance.now() - started;
        const roundtrip = await download(session);
        const importExportMs = performance.now() - started;
        assert.deepEqual(ordered(roundtrip.backup.entries), ordered([...exported.backup.entries, ...entries]));
        assert.equal(await upload(session, file), 'Imported 0 saved fields. Existing values were kept.');
        measurement.large_backups.push({entries: count, status, import_ms: importedMs, import_export_ms: importExportMs, exact_roundtrip: true});
      }
    }
    await send('Target.disposeBrowserContext', {browserContextId});
  }
  }
  const summarize = values => { const sorted = values.toSorted((a, b) => a - b); return {n: sorted.length, min: sorted[0], median: sorted[Math.ceil(sorted.length / 2) - 1], p95_nearest_rank: sorted[Math.ceil(sorted.length * .95) - 1], max: sorted.at(-1)}; };
  result.distribution = [1440, 390].map(width => { const samples = result.runs.filter(r => r.viewport[0] === width); return {width, fresh_complete_ms: summarize(samples.map(r => r.complete_ms)), warm_complete_ms: summarize(samples.map(r => r.warm_reload.complete_ms))}; });
  result.status = 'pass';
} catch (error) { failed = true; result.status = 'fail'; result.error = error.stack; }
finally { clearTimeout(deadline); await cleanup(); const json = JSON.stringify(result, null, 2); if (option('--output')) await writeFile(resolve(option('--output')), json + '\n'); console.log(json); }
process.exitCode = failed ? 1 : 0;
