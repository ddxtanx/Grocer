// Functional test of the browser parser WASM using the exact C ABI app.js uses.
const fs = require('fs');
(async () => {
  const bytes = fs.readFileSync('crates/grocer-server/static/grocer-parser.wasm');
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const m = instance.exports;

  function wasmParse(text) {
    const enc = new TextEncoder().encode(text);
    const p = m.alloc(enc.length);
    new Uint8Array(m.memory.buffer, p, enc.length).set(enc);
    const rp = m.parse(p, enc.length);
    const buf = new Uint8Array(m.memory.buffer);
    let end = rp; while (buf[end] !== 0) end++;
    const out = new TextDecoder().decode(buf.slice(rp, end));
    m.free_str(rp);
    return JSON.parse(out);
  }
  function wasmChain(text) {
    const enc = new TextEncoder().encode(text);
    const p = m.alloc(enc.length);
    new Uint8Array(m.memory.buffer, p, enc.length).set(enc);
    const rp = m.detect_chain(p, enc.length);
    const buf = new Uint8Array(m.memory.buffer);
    let end = rp; while (buf[end] !== 0) end++;
    const out = new TextDecoder().decode(buf.slice(rp, end));
    m.free_str(rp);
    return out;
  }

  const fix = f => fs.readFileSync('crates/grocer-parser/tests/fixtures/' + f, 'utf8');

  const c = wasmParse(fix('costco_652_redacted.txt'));
  console.log('costco:', c.chain, '| total', c.total_cents, c.total_source,
    '| lines', c.lines.length, '| sub', c.subtotal_cents, '| tax', c.tax_cents,
    '| member masked:', !c.raw_text.includes('111894750330'));

  const t = wasmParse(fix('traderjoes_725.txt'));
  const bulgogi = t.lines.find(l => l.raw_name === 'MEATLESS KOREAN BULGOGI');
  console.log('tj:', t.chain, '| total', t.total_cents, '| lines', t.lines.length,
    '| bulgogi qty', bulgogi.qty_count, bulgogi.line_total_cents);

  const f = wasmParse(fix('freshthyme_905.txt'));
  const rc = f.lines.find(l => l.raw_name === 'RED CABBAGE');
  console.log('ft:', f.chain, '| total', f.total_cents, '| lines', f.lines.length,
    '| cabbage', rc.qty_grams.toFixed(0), 'g @', rc.unit_price_cents, '->', rc.line_total_cents);

  console.log('chain detect:', wasmChain(fix('traderjoes_725.txt')), '|', wasmChain('hello store'));

  // hard-issue expectations (mirrors the Rust corpus test)
  const hard = c.issues.filter(i => ['SumMismatch', 'LineMathMismatch', 'TaxMismatch'].includes(i.kind));
  const hardf = f.issues.filter(i => ['SumMismatch', 'LineMathMismatch'].includes(i.kind));
  console.log('hard issues costco/ft:', hard.length, hardf.length);
  console.log('WASM_FUNCTIONAL_OK');
})().catch(e => { console.error('FAIL', e); process.exit(1); });
