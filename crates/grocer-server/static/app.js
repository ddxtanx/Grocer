'use strict';
// ---- helpers ----
const $ = s => document.querySelector(s);
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const money = c => c == null ? '' : (c / 100).toFixed(2);
const toCents = v => { const x = parseFloat(String(v).replace(/[$,]/g, '')); return Number.isFinite(x) ? Math.round(x * 100) : null; };
async function api(path, opts = {}) {
  const res = await fetch(path, { credentials: 'same-origin', headers: { 'content-type': 'application/json' }, ...opts });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.error || res.statusText);
  return body;
}
let toastT;
function toast(msg, ms = 3500) {
  const t = $('#toast'); t.textContent = msg; t.classList.add('show');
  clearTimeout(toastT); toastT = setTimeout(() => t.classList.remove('show'), ms);
}
function status(el, msg) { $(el).textContent = msg; }

let STORES = [], TRIPS = [], PRODUCTS = [];

// ---- auth ----
let authMode = 'login';
$('#authform').addEventListener('submit', e => { e.preventDefault(); doAuth(authMode); });
$('#regbtn').addEventListener('click', () => doAuth('register'));
async function doAuth(mode) {
  try {
    await api('/api/auth/' + mode, { method: 'POST', body: JSON.stringify({ username: $('#username').value, password: $('#password').value }) });
    if (mode === 'register') await api('/api/auth/login', { method: 'POST', body: JSON.stringify({ username: $('#username').value, password: $('#password').value }) });
    await boot();
  } catch (e) { toast('Auth failed: ' + e.message); }
}
async function boot() {
  const me = await api('/api/me');
  $('#auth').hidden = true; $('#tabs').hidden = false; $('main').hidden = false;
  // Security contract in this file: every value interpolated into innerHTML is
// either passed through esc() (which escapes & < > ") or is server-typed
// numeric (cents, ids) formatted through money()/String() before use.
$('#who').innerHTML = esc(me.username) + ' · <a href="#" onclick="location.reload()">switch</a>';
  STORES = await api('/api/stores');
  fillStoreSelects();
  showTab('scan');
}

// ---- tabs ----
$('#tabs').addEventListener('click', e => { if (e.target.dataset.tab) showTab(e.target.dataset.tab); });
function showTab(name) {
  document.querySelectorAll('nav button').forEach(b => b.classList.toggle('on', b.dataset.tab === name));
  document.querySelectorAll('.tab').forEach(t => t.hidden = t.id !== 'tab-' + name);
  ({ receipts: loadReceipts, fridge: loadFridge, catalog: loadCatalog, lists: loadLists, shelftag: loadShelftag }[name] || (() => {}))();
}

// ---- store selects ----
function storeOptions(sel, matchRef) {
  sel.innerHTML = '<option value="">— pick store —</option>' + STORES.map(s =>
    `<option value="${s.id}" ${s.store_ref === matchRef ? 'selected' : ''}>${esc(s.chain)} — ${esc(s.name)}${s.store_ref ? ' #' + esc(s.store_ref) : ''}</option>`).join('');
}
function fillStoreSelects() { storeOptions($('#st-store'), null); }

// ================= SCAN TAB =================
let P = null, ocrWorker = null;

$('#photo').addEventListener('change', () => { $('#ocrbtn').disabled = !$('#photo').files[0]; });
$('#ocrbtn').addEventListener('click', async () => {
  const file = $('#photo').files[0]; if (!file) return;
  try {
    status('#scanstatus', 'Starting OCR engine (~11 MB language pack, first run)…');
    ocrWorker = await Tesseract.createWorker('eng', 1, {
      workerPath: '/vendor/worker.min.js',
      corePath: '/vendor/',
      langPath: '/vendor/tessdata',
      cacheMethod: 'none',
      logger: m => status('#scanstatus', 'OCR: ' + (m.status || m.progress || '')),
    });
    const { data: { text } } = await ocrWorker.recognize(file);
    await ocrWorker.terminate(); ocrWorker = null;
    $('#rawtext').value = text;
    $('#parsebtn').disabled = !text.trim();
    status('#scanstatus', `OCR done — ${text.split('\n').length} lines. Review/fix the text, then parse.`);
  } catch (e) { status('#scanstatus', 'OCR failed: ' + e.message); }
});
// ---- parser: WASM in-browser first (grocer-parser crate, same code as the
// server's); server round-trip only as fallback if the wasm module fails to load
let WP;
async function parserWasm() {
  if (WP !== undefined) return WP;
  try {
    const { instance } = await WebAssembly.instantiateStreaming(fetch('/grocer-parser.wasm'));
    const m = instance.exports;
    const enc = new TextEncoder(), dec = new TextDecoder();
    WP = { parse(text) {
      const b = enc.encode(text);
      const p = m.alloc(b.length);
      new Uint8Array(m.memory.buffer, p, b.length).set(b);
      const rp = m.parse(p, b.length);
      const buf = new Uint8Array(m.memory.buffer);
      let end = rp; while (buf[end] !== 0) end++;
      const out = dec.decode(buf.slice(rp, end));
      m.free_str(rp);
      return JSON.parse(out);
    } };
  } catch (e) { WP = null; }
  return WP;
}
async function parseReceipt(text) {
  const w = await parserWasm();
  if (w) { status('#scanstatus', 'Parsed by in-browser WASM ✓ (image & text never left this device)'); return w.parse(text); }
  status('#scanstatus', 'WASM unavailable — parsing via server.');
  return api('/api/parse_text', { method: 'POST', body: JSON.stringify({ raw_text: text }) });
}

$('#parsebtn').addEventListener('click', async () => {
  try {
    P = await parseReceipt($('#rawtext').value);
    renderReview();
  } catch (e) { toast('Parse failed: ' + e.message); }
});

function renderReview() {
  const hard = P.issues.filter(i => ['SumMismatch', 'LineMathMismatch', 'TaxMismatch'].includes(i.kind));
  const storeSel = document.createElement('select'); storeSel.id = 'rv-store';
  storeOptions(storeSel, P.store_ref);
  $('#review').innerHTML = `
    <div class="issues"><b>${esc(P.chain)}</b> · txn ${esc(P.txn_ref || '?')} ·
      <input id="rv-when" size="20" value="${esc(P.occurred_at || '')}">
      <ul>${P.issues.map(i => `<li>${esc(i.kind)}: ${esc(i.detail)}</li>`).join('') || '<li>no issues</li>'}</ul></div>
    <div>Store: </div>
    <div>Subtotal $<input id="rv-sub" size="7" value="${money(P.subtotal_cents)}">
      Tax $<input id="rv-tax" size="6" value="${money(P.tax_cents)}">
      Total $<input id="rv-tot" size="7" value="${money(P.total_cents)}">
      <small class="mut">(${esc(P.total_source)})</small>
      <button id="rv-save">Create trip & save</button></div>
    ${hard.length ? '<p class="issues">Arithmetic problems remain — fix highlighted lines before saving.</p>' : ''}
    <table id="rv-table"><tr><th>#</th><th>Item</th><th>Code</th><th>Qty</th><th>Unit $</th><th>Line $</th><th>Save $</th><th></th></tr>
    ${P.lines.map((l, i) => `<tr class="${l.confidence < 0.7 ? 'lowconf' : ''}">
      <td>${l.seq}</td>
      <td><input data-f="raw_name" data-i="${i}" value="${esc(l.raw_name)}"></td>
      <td>${esc(l.store_item_code || '')}</td>
      <td><input data-f="qty_count" data-i="${i}" size="3" value="${l.qty_count ?? ''}"></td>
      <td><input data-f="unit" data-i="${i}" size="5" value="${money(l.unit_price_cents)}"></td>
      <td><input data-f="total" data-i="${i}" size="5" value="${money(l.line_total_cents)}"></td>
      <td>${money(l.savings_cents)}</td>
      <td><small class="mut">${esc((l.department || '') + ' ' + (l.flags || []).join(','))}</small></td></tr>`).join('')}
    </table>`;
  $('#review').insertBefore(storeSel, $('#review').querySelector('div:nth-of-type(2)'));
  $('#rv-table').addEventListener('input', e => {
    const i = +e.target.dataset.i, f = e.target.dataset.f;
    if (!Number.isInteger(i)) return;
    const l = P.lines[i];
    if (f === 'raw_name') l.raw_name = e.target.value;
    else if (f === 'qty_count') l.qty_count = e.target.value === '' ? null : parseInt(e.target.value, 10);
    else if (f === 'unit') l.unit_price_cents = toCents(e.target.value);
    else if (f === 'total') l.line_total_cents = toCents(e.target.value) ?? 0;
  });
  $('#rv-save').addEventListener('click', saveReceipt);
}

async function saveReceipt() {
  const sid = parseInt($('#rv-store').value, 10);
  if (!sid) return toast('Pick the store first.');
  P.subtotal_cents = toCents($('#rv-sub').value);
  P.tax_cents = toCents($('#rv-tax').value);
  P.total_cents = toCents($('#rv-tot').value);
  P.occurred_at = $('#rv-when').value || null;
  try {
    const trip = await api('/api/trips', { method: 'POST', body: JSON.stringify({ store_id: sid, occurred_at: P.occurred_at || new Date().toISOString() }) });
    const out = await api('/api/receipts', { method: 'POST', body: JSON.stringify({ trip_id: trip.id, store_id: sid, receipt: P }) });
    toast(out.deduplicated ? `Already scanned (receipt #${out.receipt_id})` :
      `Saved receipt #${out.receipt_id}` + (out.issues.length ? ` — notes: ${out.issues.join(' | ')}` : ''));
    P = null; $('#review').innerHTML = '';
  } catch (e) { toast('Rejected: ' + e.message, 8000); }
}

// ================= RECEIPTS TAB =================
async function loadReceipts() {
  const rs = await api('/api/receipts');
  $('#receiptlist').innerHTML = '<table><tr><th>#</th><th>When</th><th>Store</th><th>Total</th><th></th></tr>' +
    rs.map(r => `<tr><td>${r.id}</td><td>${esc(r.occurred_at || '')}</td><td>${esc(r.store)}</td>
      <td>$${money(r.total_cents)}</td><td><button data-open="${r.id}">open</button></td></tr>`).join('') + '</table>';
  $('#receiptlist').onclick = async e => {
    if (!e.target.dataset.open) return;
    const d = await api('/api/receipts/' + e.target.dataset.open);
    $('#receiptdetail').innerHTML = `<h3>Receipt #${d.id} — $${money(d.total_cents)} <small class="mut">(${esc(d.total_source)})</small></h3>
      <table><tr><th>Item</th><th>Code</th><th>Qty</th><th>Unit</th><th>Line</th><th>Product</th></tr>
      ${d.items.map(i => `<tr><td>${esc(i.name)}</td><td>${esc(i.store_item_code || '')}</td>
        <td>${i.qty ?? ''}</td><td>$${money(i.unit_price_cents)}</td><td>$${money(i.line_total_cents)}</td>
        <td>${i.product_id ? '#' + i.product_id : `<input placeholder="product id" data-link="${i.id}" size="8"><button data-linkbtn="${i.id}">link</button>`}</td></tr>`).join('')}</table>`;
    $('#receiptdetail').onclick = async ev => {
      const id = ev.target.dataset.linkbtn; if (!id) return;
      const inp = $(`input[data-link="${id}"]`);
      const pid = parseInt(inp.value, 10); if (!pid) return toast('Enter a product id');
      await api(`/api/receipt_items/${id}/link`, { method: 'POST', body: JSON.stringify({ product_id: pid }) });
      toast('Linked'); loadReceipts();
    };
  };
}

// ================= SHELF TAG TAB =================
let stAdvertised = null, barStream = null;
async function loadShelftag() {
  TRIPS = await api('/api/trips');
  $('#st-trip').innerHTML = '<option value="">— none —</option>' +
    TRIPS.map(t => `<option value="${t.id}">${esc(t.store)} @ ${esc(t.occurred_at)}</option>`).join('');
}
$('#st-scanbar').addEventListener('click', async () => {
  if (!('BarcodeDetector' in window)) return toast('No BarcodeDetector in this browser — type the UPC.');
  const det = new BarcodeDetector({ formats: ['ean_13', 'upc_a', 'upc_e'] });
  barStream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } });
  const v = $('#st-video'); v.hidden = false; v.srcObject = barStream; await v.play();
  const tick = async () => {
    if (!barStream) return;
    try {
      const codes = await det.detect(v);
      if (codes.length) { $('#st-upc').value = codes[0].rawValue; stopBar(); toast('Barcode: ' + codes[0].rawValue); }
    } catch {}
    requestAnimationFrame(tick);
  };
  const stopBar = () => { barStream.getTracks().forEach(t => t.stop()); barStream = null; v.hidden = true; };
  tick();
});
$('#st-photo').addEventListener('change', async () => {
  const file = $('#st-photo').files[0]; if (!file) return;
  try {
    status('#st-status', 'OCR’ing tag…');
    const w = await Tesseract.createWorker('eng', 7, {
      workerPath: '/vendor/worker.min.js', corePath: '/vendor/', langPath: '/vendor/tessdata',
      cacheMethod: 'none', logger: m => status('#st-status', 'tag OCR: ' + (m.status || '')),
    });
    const { data: { text } } = await w.recognize(file);
    await w.terminate();
    const seen = new Set();
    for (const m of text.matchAll(/\$\s?(\d{1,3}(?:,\d{3})*(?:\.\d\d))|\b(\d{1,3}\.\d\d)\b/g)) {
      const c = toCents(m[1] || m[2]); if (c && c > 1) seen.add(c);
    }
    stAdvertised = null;
    $('#st-cands').innerHTML = 'Pick the advertised price: ' + [...seen].sort((a, b) => a - b).map(c =>
      `<button data-adv="${c}">$${money(c)}</button>`).join(' ') || '<b>No prices found — retake photo.</b>';
    status('#st-status', `${seen.size} price candidate(s) from tag text.`);
  } catch (e) { status('#st-status', 'OCR failed: ' + e.message); }
});
$('#st-cands').addEventListener('click', e => {
  if (!e.target.dataset.adv) return;
  stAdvertised = parseInt(e.target.dataset.adv, 10);
  [...$('#st-cands').children].forEach(b => b.style.outline = b === e.target ? '3px solid #222' : '');
});
$('#st-save').addEventListener('click', async () => {
  if (!stAdvertised) return toast('Pick the advertised price first.');
  const body = { store_id: parseInt($('#st-store').value, 10), upc: $('#st-upc').value || null,
    advertised_cents: stAdvertised, trip_id: $('#st-trip').value ? parseInt($('#st-trip').value, 10) : null };
  if (!body.store_id) return toast('Pick a store.');
  const out = await api('/api/price-checks', { method: 'POST', body: JSON.stringify(body) });
  toast(`Shelf price recorded (check #${out.id}, product ${out.resolved_product_id ?? 'unresolved'})`);
  if (body.upc) {
    const hist = await api('/api/prices?upc=' + encodeURIComponent(body.upc));
    $('#st-history').innerHTML = hist.length ? '<h4>Known prices for this UPC</h4><table><tr><th>Store</th><th>Unit</th><th>When</th><th>Src</th></tr>' +
      hist.map(h => `<tr><td>${esc(h.store)}</td><td>$${money(h.unit_price_cents)}</td><td>${esc(h.observed_at)}</td><td>${esc(h.source)}</td></tr>`).join('') + '</table>' : '';
  }
});
$('#st-integrity').addEventListener('click', async () => {
  const t = $('#st-trip').value; if (!t) return toast('Pick the trip that has its receipt scanned.');
  const v = await api(`/api/trips/${t}/price-integrity`);
  $('#st-verdicts').innerHTML =
    (v.verdicts.length ? '<table><tr><th>Product</th><th>Advertised</th><th>Charged</th><th>Verdict</th></tr>' +
      v.verdicts.map(x => `<tr><td>${esc(x.product || '?')}</td><td>$${money(x.advertised_cents)}</td>
        <td>$${money(x.charged_cents)}</td><td class="verdict-${x.verdict}">${x.verdict}</td></tr>`).join('') + '</table>' : '<i>No comparable receipt lines yet — scan that trip’s receipt first.</i>') +
    (v.unmatched_checks.length ? `<p class="mut">No receipt line found for: ${v.unmatched_checks.map(esc).join(', ')}</p>` : '');
});

// ================= FRIDGE TAB =================
async function loadFridge() {
  const rows = await api('/api/fridge/expiring?days=7');
  $('#fridge-expiring').innerHTML = rows.length ? '<table><tr><th>Product</th><th>Qty</th><th>Days left</th></tr>' +
    rows.map(f => `<tr><td>${esc(f.product)}</td><td>${f.qty}</td><td>${f.days_left}</td></tr>`).join('') + '</table>' :
    '<i>Nothing expiring within 7 days.</i>';
  $('#fridge-expiring').dataset.ids = JSON.stringify(rows.map(f => f.product_id));
}
$('#restockbtn').addEventListener('click', async () => {
  const ids = JSON.parse($('#fridge-expiring').dataset.ids || '[]');
  if (!ids.length) return toast('Nothing to restock.');
  const list = await api('/api/lists', { method: 'POST', body: JSON.stringify({ name: 'Restock ' + new Date().toISOString().slice(0, 10) }) });
  for (const pid of ids) await api(`/api/lists/${list.id}/items`, { method: 'POST', body: JSON.stringify({ product_id: pid, qty: 1 }) });
  toast(`${ids.length} items → list #${list.id}. Open Lists to plan the run.`);
});
$('#fa-search').addEventListener('input', async () => {
  PRODUCTS = await api('/api/products?q=' + encodeURIComponent($('#fa-search').value));
  $('#fa-prodl').innerHTML = PRODUCTS.map(p => `<option value="${esc(p.name)}" data-id="${p.id}">`).join('');
});
$('#fa-add').addEventListener('click', async () => {
  const opt = [...$('#fa-prodl').options].find(o => o.value === $('#fa-search').value);
  if (!opt) return toast('Pick a product from the suggestions.');
  const body = { product_id: parseInt(opt.dataset.id, 10), qty: parseInt($('#fa-qty').value, 10) || 1 };
  if ($('#fa-days').value) body.expires_in_days = parseInt($('#fa-days').value, 10);
  await api('/api/fridge', { method: 'POST', body: JSON.stringify(body) });
  toast('Added to fridge'); loadFridge();
});

// ================= LISTS TAB =================
let LS = null;
async function loadLists() {
  if (!LS) { $('#ls-body').innerHTML = '<i>Create or pick a list above.</i>'; return; }
  const items = LS.items || [];
  $('#ls-body').innerHTML = `<h3>List #${LS.id} “${esc(LS.name)}”</h3>
    <input id="li-q" placeholder="search product…" list="li-prodl" size="18"><datalist id="li-prodl"></datalist>
    <input id="li-qty" type="number" value="1" min="1" style="width:4em">
    <button id="li-add">Add</button> <button id="li-plan">Plan trip →</button>
    <table><tr><th>Product</th><th>Qty</th></tr>${items.map(i =>
      `<tr><td>${esc(i.name)}</td><td>${i.qty}</td></tr>`).join('') || '<tr><td colspan="2"><i>empty</i></td></tr>'}</table>
    <div id="ls-plan"></div>`;
  $('#li-q').addEventListener('input', async () => {
    PRODUCTS = await api('/api/products?q=' + encodeURIComponent($('#li-q').value));
    $('#li-prodl').innerHTML = PRODUCTS.map(p => `<option value="${esc(p.name)}" data-id="${p.id}">`).join('');
  });
  $('#li-add').addEventListener('click', async () => {
    const opt = [...$('#li-prodl').options].find(o => o.value === $('#li-q').value);
    if (!opt) return toast('Pick a product.');
    const qty = parseInt($('#li-qty').value, 10) || 1;
    await api(`/api/lists/${LS.id}/items`, { method: 'POST', body: JSON.stringify({ product_id: parseInt(opt.dataset.id, 10), qty }) });
    LS.items.push({ name: opt.value, qty }); loadLists();
  });
  $('#li-plan').addEventListener('click', async () => {
    try {
      const p = await api(`/api/plans/${LS.id}`, { method: 'POST' });
      const byStore = {};
      p.items.forEach(i => (byStore[i.store] ||= []).push(i));
      $('#ls-plan').innerHTML = `<h4>Plan #${p.plan_id}: ${p.stops} stop(s), est. $${money(p.total_cents)}</h4>` +
        Object.entries(byStore).map(([st, its]) => `<div class="planstop"><b>${esc(st)}</b><ul>` +
          its.map(i => `<li>${esc(i.product)} ×${i.qty} @ $${money(i.unit_price_cents)}</li>`).join('') + '</ul></div>').join('') +
        (p.unpriced.length ? `<p class="mut">No price data yet for: ${p.unpriced.map(esc).join(', ')}</p>` : '');
    } catch (e) { toast('Plan: ' + e.message); }
  });
}
$('#ls-new').addEventListener('click', async () => {
  const name = $('#ls-name').value || 'List';
  const l = await api('/api/lists', { method: 'POST', body: JSON.stringify({ name }) });
  LS = { id: l.id, name, items: [] }; loadLists();
});

// ================= CATALOG TAB =================
async function loadCatalog() {
  const rows = await api('/api/items/unlinked');
  PRODUCTS = await api('/api/products');
  const pname = {}; PRODUCTS.forEach(p => pname[p.id] = p.name);
  $('#cat-unlinked').innerHTML = rows.length ? '<table><tr><th>Raw name</th><th>Chain</th><th>Code</th><th>Unit</th><th>Link</th></tr>' +
    rows.map(r => {
      const cands = (r.candidates || []).map(c =>
        `<button data-linkto="${r.item_id}" data-pid="${c.product_id}">${esc(pname[c.product_id] || '#' + c.product_id)} (${c.confirmations}✓)</button>`).join(' ');
      return `<tr><td>${esc(r.raw_name)}</td><td>${esc(r.chain)}</td><td>${esc(r.store_item_code || '')}</td>
        <td>$${money(r.unit_price_cents)}</td>
        <td>${cands}<input placeholder="or product id" size="8" data-cpid="${r.item_id}"><button data-linkcustom="${r.item_id}">link</button></td></tr>`;
    }).join('') + '</table>' : '<i>Everything is linked. 🎉</i>';
  $('#cat-unlinked').onclick = async e => {
    let item, pid;
    if (e.target.dataset.linkto) { item = e.target.dataset.linkto; pid = e.target.dataset.pid; }
    else if (e.target.dataset.linkcustom) {
      item = e.target.dataset.linkcustom;
      pid = $(`input[data-cpid="${item}"]`).value;
    } else return;
    if (!pid) return toast('Product id required');
    await api(`/api/receipt_items/${item}/link`, { method: 'POST', body: JSON.stringify({ product_id: parseInt(pid, 10) }) });
    toast('Linked + mapping confirmed'); loadCatalog();
  };
}
$('#cn-create').addEventListener('click', async () => {
  const body = { name: $('#cn-name').value, brand: $('#cn-brand').value || null, category: $('#cn-cat').value || null,
    upc: $('#cn-upc').value || null, pack_size: $('#cn-size').value ? parseFloat($('#cn-size').value) : null,
    size_unit: $('#cn-unit').value || null };
  if (!body.name) return toast('Name required');
  const p = await api('/api/products', { method: 'POST', body: JSON.stringify(body) });
  toast(`Product #${p.id} created`); loadCatalog();
});

// ---- auto-login if session is alive ----
api('/api/me').then(boot).catch(() => {});
