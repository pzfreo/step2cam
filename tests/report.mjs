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
const options = { library: { machines: [{ id: 'mill', name: 'Test mill', spindle: { manual_collets: true } }] }, rules: new Map([['hold@1', { rationale: 'Holding rule', sources: [] }]]), date: '09/11/2026' };
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


assert.equal(featureName({ family: 'holes' }), '⌀— hole');
assert.equal(operationName({ kind: 'counterbore' }, { id: 'top' }, data.plan), '⌀— counterbore');

// Regression coverage for appendix size: retain every decision and distinct rule,
// but print rule rationale/sources once and never dump geometric record JSON.
const detailed = structuredClone(data);
detailed.analysis.features.push(
  { id: 'cut', family: 'slots', record: { width: 3, rawGeometrySentinel: 'do not dump this geometry' } },
  { id: 'rough', family: 'pockets', record: { width: 6, depth: 2 } },
);
detailed.plan.setups[0].operations[0].technology.peck_mm = 2;
detailed.plan.setups[0].operations[0].technology.peck_mode = 'full_retract';
detailed.plan.covered_by_roughing = ['rough'];
detailed.plan.setups[0].operations[0].features = ['cut'];
detailed.plan.setups[0].operations[1].features = ['cut'];
detailed.plan.setups[0].operations[0].because.push('tool-choice');
detailed.plan.decisions.push(
  { id: 'tool-choice', subject: 'top:facing', stage: 'tools', rule: 'hold@1', says: 'Use a', source: 'rule', chosen: 'a', considered: [{ candidate: 'a' }, { candidate: 'rejected-tool', rejected: 'long rejection explanation' }] },
  { id: 'agent-choice', subject: 'part', stage: 'setup', rule: 'agent@1', says: 'Agent decision', source: 'agent:reviewer', reason: 'Agent reason', superseded_by: 'warning' },
  { id: 'missing-rule', subject: 'part', stage: 'setup', rule: 'unknown@1', says: 'Unknown rule decision', source: 'rule' },
);
detailed.plan.workholding = { source: 'rule', candidates: [
  { id: 'ordinary', label: 'Ordinary grip', available: false, reason: 'Needs a tool', tools_needed: detailed.plan.tools_needed },
  { id: 'flip', label: 'Flip option', available: false, reason: 'Needs a tool', tools_needed: detailed.plan.tools_needed },
] };
const compactOptions = { ...options, rules: new Map([...options.rules, ['agent@1', { rationale: 'Agent rule text', sources: [{ kind: 'standard', ref: 'Test standard' }] }]]) };
const compact = reportHtml(detailed, compactOptions);
const sectionTable = (name) => compact.match(new RegExp(`<table class="${name}">([\\s\\S]*?)</table>`))[1];
const decisionTable = sectionTable('decisions');
assert.equal((decisionTable.match(/<tbody><tr>|<\/tr><tr>/g) || []).length, detailed.plan.decisions.length, 'one row per decision');
for (const d of detailed.plan.decisions) assert.ok(decisionTable.includes(d.id) && decisionTable.includes(d.says) && decisionTable.includes(d.rule), `decision retained: ${d.id}`);
assert.ok(!decisionTable.includes('<p>—</p>'), 'no empty considered-list placeholders');
assert.ok(decisionTable.includes('Chose a over rejected-tool.'));
assert.ok(!compact.includes('long rejection explanation'), 'only brief alternatives retained');
assert.ok(decisionTable.includes('user: shop constraint') && decisionTable.includes('agent:reviewer: Agent reason'));
assert.ok(decisionTable.includes('Replaced by warning.'));
const rulesTable = sectionTable('rules');
assert.equal((rulesTable.match(/<tbody><tr>|<\/tr><tr>/g) || []).length, new Set(detailed.plan.decisions.map(d => d.rule)).size);
for (const phrase of ['Holding rule', 'Agent rule text', 'Test standard']) assert.equal(compact.split(phrase).length - 1, 1, `${phrase} printed once`);
assert.ok(rulesTable.includes('unknown@1') && rulesTable.includes('Rule text unavailable.'));
const featureTable = sectionTable('features');
for (const phrase of ['Facing', 'Spot drill', 'covered by roughing', 'not cut']) assert.ok(featureTable.includes(phrase));
assert.ok(!compact.includes('rawGeometrySentinel') && !compact.includes('do not dump this geometry'));
const setupSheet = compact.split('<section class="appendix">')[0];
assert.ok(setupSheet.includes('Peck depth: 2 mm') && setupSheet.includes('Peck mode: full_retract'));
assert.ok(setupSheet.includes('Tool choice: ✓ a — the only one that qualifies'));
assert.ok(!setupSheet.includes('rejected-tool') && !setupSheet.includes('Tool choice: ✗'));
const missingTools = setupSheet.split('<h2>Tools needed but not in the library</h2>')[1].split('<section class="setup">')[0];
assert.equal(missingTools.split('special drill').length - 1, 1, 'deduplicate tool needs across plan and candidates');
for (const context of ['Current plan', 'Ordinary grip', 'Flip option']) assert.ok(missingTools.includes(context));
assert.ok(compact.includes('Flute length mm') && !compact.includes('Flutes mm'));
console.log('Report content, compact appendix, escaping, missing fields, ordering, collet changes and repeatability passed.');
