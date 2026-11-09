// Content and escaping checks; no output hashes or dependency on a browser.
import assert from 'node:assert/strict';
import { reportDocument, reportLength, reportDuration, sheetWords, checkText, featureName, operationName, NOT_FEATURES } from '../demo/report.js';
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
// Assertions now inspect model values, not HTML markup: the PDF renders these
// exact strings as text, so filenames must remain unescaped and rows stay arrays.
const reportText = (report) => report.blocks.map(block => block.type === 'table' ? [block.headers, ...block.rows].map(row => row.join(' | ')).join('\n') : block.type === 'list' ? block.items.join('\n') : block.text || '').join('\n');
const tableText = (table) => table.rows.map(row => row.join(' | ')).join('\n');
const document = reportDocument(data, options);
const html = reportText(document);
for (const phrase of ['SETUP SHEET', 'APPENDIX — FULL REPORT', 'NOT CUT', 'WARNINGS', 'special drill', 'Holding rule', 'shop constraint', 'Manual collet change', '5000', '600', '100', '1 / 3', '0:06', '20 × 30 × 10', 'T7', 'T2', '— / —', 'not compared with the part', 'Uncertain-motion']) {
  assert.ok(html.toLowerCase().includes(phrase.toLowerCase()), `missing ${phrase}`);
}
assert.equal(document.title, data.name, 'PDF model must preserve literal filename characters');
assert.ok(html.includes(data.name) && !html.includes('&lt;part'), 'PDF text is not HTML-encoded');
assert.ok(html.indexOf('T7') < html.indexOf('T2'), 'tools follow first use, not T number');
assert.equal(document.blocks.filter(b => b.type === 'heading' && b.text.startsWith('SETUP —')).length, 1);
assert.ok(!html.includes('undefined') && !html.includes('NaN'));
assert.deepEqual(checkText([{ setup: 'top' }]).compact, '⚠ unchecked');
assert.deepEqual(reportDocument(data, options), document, 'fixed inputs and date produce fixed content');


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
const compactDocument = reportDocument(detailed, compactOptions);
const compact = reportText(compactDocument);
const sectionTable = (name) => compactDocument.blocks.find(block => block.type === 'table' && block.id === name);
const decisionRows = sectionTable('decisions').rows;
const decisionTable = tableText(sectionTable('decisions'));
assert.equal(decisionRows.length, detailed.plan.decisions.length, 'one row per decision');
for (const d of detailed.plan.decisions) assert.ok(decisionTable.includes(d.id) && decisionTable.includes(d.says) && decisionTable.includes(d.rule), `decision retained: ${d.id}`);
assert.ok(decisionRows.every(row => row.length === 3 && !row.includes('—')), 'no empty considered-list placeholders');
assert.ok(decisionTable.includes('Chose a over rejected-tool.'));
assert.ok(!compact.includes('long rejection explanation'), 'only brief alternatives retained');
assert.ok(decisionTable.includes('user: shop constraint') && decisionTable.includes('agent:reviewer: Agent reason'));
assert.ok(decisionTable.includes('Replaced by warning.'));
const rulesTable = tableText(sectionTable('rules'));
assert.equal(sectionTable('rules').rows.length, new Set(detailed.plan.decisions.map(d => d.rule)).size);
for (const phrase of ['Holding rule', 'Agent rule text', 'Test standard']) assert.equal(compact.split(phrase).length - 1, 1, `${phrase} printed once`);
assert.ok(rulesTable.includes('unknown@1') && rulesTable.includes('Rule text unavailable.'));
const featureTable = tableText(sectionTable('features'));
for (const phrase of ['Facing', 'Spot drill', 'covered by roughing', 'not cut']) assert.ok(featureTable.includes(phrase));
assert.ok(!compact.includes('rawGeometrySentinel') && !compact.includes('do not dump this geometry'));
const sheetBlocks = compactDocument.blocks.slice(0, compactDocument.appendixStart);
const setupSheet = reportText({ blocks: sheetBlocks });
assert.ok(setupSheet.includes('Peck depth: 2 mm') && setupSheet.includes('Peck mode: full-retract pecks'));
assert.ok(setupSheet.includes('Tool choice: ✓ a — the only one that qualifies'));
assert.ok(!setupSheet.includes('rejected-tool') && !setupSheet.includes('Tool choice: ✗'));
const missingHeading = sheetBlocks.findIndex(b => b.text === 'Tools needed but not in the library');
const missingTools = sheetBlocks[missingHeading + 1].items.join('\n');
assert.equal(missingTools.split('special drill').length - 1, 1, 'deduplicate tool needs across plan and candidates');
for (const context of ['Current plan', 'Ordinary grip', 'Flip option']) assert.ok(missingTools.includes(context));
assert.ok(compact.includes('Flute length mm') && !compact.includes('Flutes mm'));
// Whole-part descriptions are not machining features, whether the report builds
// its feature map or receives the page's filtered and annotated map.
const grouped = structuredClone(data);
grouped.analysis.features.push(
  { id: 'pattern-description', family: 'hole_patterns', record: {} },
  { id: 'plate-description', family: 'plates', record: {} },
  { id: 'narrow', family: 'slots', record: { width: 2 } },
  { id: 'rough-note', family: 'pockets', record: { width: 6, depth: 2 } },
);
grouped.plan.skipped = [];
grouped.plan.setups[0].operations[1].features = ['hole'];
const pageFeatures = new Map(grouped.analysis.features.filter(f => !NOT_FEATURES.has(f.family))
  .map(f => [f.id, { ...f, name: featureName(f) }]));
pageFeatures.get('narrow').note = 'narrower than the ⌀6 mill, so not cut to size';
pageFeatures.get('rough-note').note = 'roughed, keeps the roughing allowance';
for (const features of [undefined, pageFeatures]) {
  const report = reportDocument(grouped, { ...options, features });
  const recognisedTable = report.blocks.find(b => b.type === 'table' && b.id === 'features');
  const recognised = tableText(recognisedTable);
  assert.ok(!recognised.includes('pattern-description') && !recognised.includes('Hole patterns'));
  assert.ok(!recognised.includes('plate-description') && !recognised.includes('Plates'));
  const holeRow = recognisedTable.rows.find(row => row[0] === 'hole').join(' | ');
  assert.ok(holeRow.includes('Spot drill') && !holeRow.includes('not cut'), 'drilled hole retains its operation');
  assert.equal(recognisedTable.rows.length, pageFeatures.size);
  if (features) for (const id of ['narrow', 'rough-note']) {
    assert.ok(recognised.includes(pageFeatures.get(id).note), `retain page status for ${id}`);
  }
}
// Display changes are intentional: fractional lengths have two decimals except
// nominal 1/8-inch geometry (3.175 mm), rates are whole numbers, JSON stays out
// of the machinist's sheet, and absent tool needs produce no empty section.
assert.equal(reportLength(11.8364), '11.84');
assert.equal(reportLength(6.343107), '6.34');
assert.equal(reportLength(3.175), '3.175');
assert.equal(reportDuration((22 * 60 + 38) * 60 + 16), '22 h 38 min');
assert.equal(sheetWords('Closed_slot with Radiused_slot_end_type ends; bottom_and_side_finish_milling; Vee_profile; full_retract; unknown_identifier'), 'slot with rounded ends; finish bottom and sides; V-channel; full-retract pecks; unknown identifier');
assert.equal(sheetWords('Check cut {"detail":{"bad":[1,2]}} before cutting'), 'Check cut before cutting');
const tidy = structuredClone(data);
tidy.tools[0].diameter = 3.175;
tidy.tools[0].flute_length = 11.8364;
tidy.plan.setups[0].operations[0].technology.feed_mm_min = 304.8;
tidy.plan.setups[0].operations[0].technology.spindle_rpm = 257.18;
tidy.plan.tools_needed = [];
const tidyDoc = reportDocument(tidy, options);
assert.ok(!tidyDoc.blocks.some(b => b.text === 'Tools needed but not in the library'));
const toolsRow = tidyDoc.blocks.find(b => b.id === 'tools').rows[0];
assert.equal(toolsRow[1], 'end mill / 3.175');
assert.equal(toolsRow[2], '11.84');
assert.equal(toolsRow[4], '257');
assert.equal(toolsRow[5], '305');
const collets = tidyDoc.blocks.find(b => b.text?.startsWith('Collet changes —'));
assert.ok(collets.text.includes('top: 6, 3 mm shanks'));
assert.ok(tidyDoc.blocks.indexOf(collets) < tidyDoc.blocks.findIndex(b => b.type === 'page-break'));
assert.equal(tidy.plan.setups[0].operations[0].technology.feed_mm_min, 304.8, 'formatting never mutates plan data');

const actualColletSchema = reportDocument(tidy, { ...options, library: { machines: [{ id: 'mill', spindle: { collet_change: 'manual' } }] } });
assert.ok(actualColletSchema.blocks.some(b => b.text?.startsWith('Collet changes —')));
const tracedCollets = structuredClone(tidy);
tracedCollets.plan.decisions.push({ id: 'collets', stage: 'machine', outcome: 'warning', source: 'rule', rule: 'machine.spindle.manual_collets@1', subject: 'top', shank_sizes_mm: [3.175, 6.35], says: 'Collets must be changed by hand.' });
const colletTraceReport = reportDocument(tracedCollets, { ...options, library: {} });
assert.ok(colletTraceReport.blocks.find(b => b.text?.startsWith('Collet changes —')).text.includes('top: 3.175, 6.35 mm shanks'));
assert.ok(reportText({ blocks: colletTraceReport.blocks.slice(0, colletTraceReport.blocks.findIndex(b => b.type === 'page-break')) }).includes('Collets must be changed by hand.'));
const nearMinute = structuredClone(data);
nearMinute.programs[0].moves[0].points[1] = [596, 0, 0];
assert.equal(reportDocument(nearMinute, options).blocks.find(b => b.id === 'operations').rows[0][7], '1:00', 'rounded operation seconds carry into the minute');
console.log('Report model, display formatting, compact appendix, feature filtering/notes, escaping, missing fields, ordering, collet changes and repeatability passed.');
