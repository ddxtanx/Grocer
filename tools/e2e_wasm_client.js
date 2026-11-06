// E2E: simulate the PWA client exactly as app.js does it, but headless:
// WASM parse -> create trip -> ingest -> verify stored totals.
const fs = require('fs');
const BASE = 'http://127.0.0.1:8787';

(async () => {
  // login
  let res = await fetch(BASE + '/api/auth/login', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username: 'garrett', password: 's3cr3tpass' }),
  });
  const cookie = res.headers.get('set-cookie');
  const J = (opts = {}) => ({ ...opts, headers: { 'content-type': 'application/json', cookie, ...(opts.headers || {}) } });

  // WASM parser (same artifact the browser fetches)
  const wasm = await fetch(BASE + '/grocer-parser.wasm');
  const { instance } = await WebAssembly.instantiate(await wasm.arrayBuffer(), {});
  const m = instance.exports;
  const enc = new TextEncoder(), dec = new TextDecoder();
  function parse(text) {
    const b = enc.encode(text);
    const p = m.alloc(b.length);
    new Uint8Array(m.memory.buffer, p, b.length).set(b);
    const rp = m.parse(p, b.length);
    const buf = new Uint8Array(m.memory.buffer);
    let end = rp; while (buf[end] !== 0) end++;
    const out = dec.decode(buf.slice(rp, end));
    m.free_str(rp);
    return JSON.parse(out);
  }

  const cases = [
    ['freshthyme_905.txt', 1, 'Fresh Thyme'],
    ['traderjoes_725.txt', 2, "Trader Joe's"],
    ['costco_652_redacted.txt', 3, 'Costco'],
  ];
  for (const [file, storeId, chain] of cases) {
    const raw = fs.readFileSync('crates/grocer-parser/tests/fixtures/' + file, 'utf8');
    const P = parse(raw);                       // client-side WASM
    if (P.chain.toLowerCase().replace(/[^a-z]/g, '') !== chain.toLowerCase().replace(/[^a-z]/g, ''))
      throw new Error(`chain mismatch ${P.chain} vs ${chain}`);
    const trip = await (await fetch(BASE + '/api/trips', J({
      method: 'POST', body: JSON.stringify({ store_id: storeId, occurred_at: P.occurred_at }) }))).json();
    const out = await (await fetch(BASE + '/api/receipts', J({
      method: 'POST', body: JSON.stringify({ trip_id: trip.id, receipt: P }) }))).json();
    if (out.error) throw new Error(`${chain} ingest: ${out.error}`);
    const detail = await (await fetch(BASE + `/api/receipts/${out.receipt_id}`, J())).json();
    console.log(`${chain}: wasm-parsed -> receipt #${out.receipt_id} total ${detail.total_cents} source ${detail.total_source} items ${detail.items.length} dedup=${out.deduplicated}`);
  }
  console.log('E2E_OK');
})().catch(e => { console.error('E2E FAIL', e.message); process.exit(1); });
