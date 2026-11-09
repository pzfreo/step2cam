// Content and escaping checks; no output hashes or dependency on a browser.
import assert from 'node:assert/strict';
import { reportHtml, checkText, featureName, operationName } from '../demo/report.js';
const data = {
  name: '<part & "test">.step', library: { machine: 'mill' }, tools: [
    { id: 'a', number: 7, type: 'end_mill', diameter: 6, shank_diameter: 6 },
    { id: 'b', number: 2, type: 'drill', diameter: 3, shank_diameter: 3 },
  ], analysis: { features: [{ id: 'hole', family: 'holes', record: { diameter: 4 } }] },
  plan: { material: { used: 'Al' }, setups: [{ id: 'top', stock: { min: [0, 0, 0], max: [20, 30, 10] }, zero: 'stock', operations: [
    { kind: 'facing', tool: 'a', because: ['warning'], technology: { spindle_rpm: 5000, feed_mm_min: 600, plunge_mm_min: 100, stepdown_mm: 1, stepover_mm: 3 } },
    { kind: 'spot_drill', tool: 'b', technology: { spindle_rpm: 4000 } },
  ] }], skipped: [{ feature: 'hole', reason: 'needs a special drill' }],
  tools_needed: [{ description: 'special drill', features: ['hole'] }], warnings: ['Check holding'],
  decisions: [{ id: 'warning', stage: 'setup', rule: 'hold@1', outcome: 'warning', source: 'user', reason: 'shop constraint', says: 'Check holding' }],
  }, programs: [{ setup: 'top', moves: [{ op: 'top-1-facing', kind: 'cut', feed: 600, points: [[0,0,0], [60,0,0]] }] }],
};
const options = { library: { machines: [{ id: 'mill', name: 'Test mill', spindle: { manual_collets: true } }] }, rules: new Map([['hold@1', { statement: 'Holding rule', sources: [] }]]), date: '09/11/2026' };
const html = reportHtml(data, options);
for (const phrase of ['SETUP SHEET', 'APPENDIX — FULL REPORT', 'NOT CUT', 'WARNINGS', 'special drill', 'Holding rule', 'shop constraint', 'Manual collet change', '5000', '600', '100', '1 / 3', '0:06', '20 × 30 × 10', 'T7', 'T2', '— / —', 'not compared with the part', 'Uncertain-motion']) {
  assert.ok(html.toLowerCase().includes(phrase.toLowerCase()), `missing ${phrase}`);
}
assert.ok(!html.includes(data.name), 'filename must be escaped');
assert.ok(html.includes('&lt;part &amp; &quot;test&quot;&gt;.step'));
assert.ok(html.indexOf('T7') < html.indexOf('T2'), 'tools follow first use, not T number');
assert.equal((html.match(/class="setup"/g) || []).length, 1);
assert.ok(!html.includes('undefined') && !html.includes('NaN'));
assert.deepEqual(checkText([{ setup: 'top' }]).compact, '⚠ unchecked');
assert.equal(reportHtml(data, options), html, 'fixed inputs and date produce fixed content');
console.log('Report content, missing fields, escaping, ordering, collet changes and repeatability passed.');

assert.equal(featureName({ family: 'holes' }), '⌀— hole');
assert.equal(operationName({ kind: 'counterbore' }, { id: 'top' }, data.plan), '⌀— counterbore');
