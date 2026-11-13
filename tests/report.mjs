// Content and escaping checks; no output hashes or dependency on a browser.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { reportDocument, reportLength, reportDuration, sheetWords, checkText, featureName, operationName, operationTime, setupDetails, workholdingText, setupZero, NOT_FEATURES, toolChoiceRows } from '../demo/report.js';
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
for (const phrase of ['SETUP SHEET', 'APPENDIX — FULL REPORT', 'NOT CUT', 'WARNINGS', 'special drill', 'Holding rule', 'shop constraint', 'Manual collet change', '5000', '600', '100', '1 / 3', '0 min 6 s', '20 × 30 × 10', 'T7', 'T2', '— / —', 'not compared with the part', 'Uncertain-motion']) {
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
// Every operation kind the plan schema allows has its own name, not the fallback.
for (const { properties: { kind: { const: kind } } } of JSON.parse(readFileSync(new URL('../schemas/plan-1.schema.json', import.meta.url))).$defs.operation.oneOf) {
  assert.ok(operationName.toString().includes(`op.kind === "${kind}"`), `operationName has no case for ${kind}`);
}

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
assert.ok(compact.slice(compact.indexOf('APPENDIX — FULL REPORT')).includes('a: the only one that qualifies'), 'chosen tool rationale stays in appendix');
assert.ok(!setupSheet.includes('Tool choice:'), 'machinist sheet omits selection rationale');
assert.ok(!setupSheet.includes('rejected-tool') && !setupSheet.includes('Tool choice: ✗'));
const missingHeading = sheetBlocks.findIndex(b => b.text === 'Tools needed but not in the library');
const missingTools = sheetBlocks[missingHeading + 1].items.join('\n');
assert.equal(missingTools.split('special drill').length - 1, 1, 'current-plan tool needs are deduplicated');
assert.ok(missingTools.includes('Current plan'));
for (const context of ['Ordinary grip', 'Flip option']) {
  assert.ok(!missingTools.includes(context), 'option needs stay out of the tool-loading list');
  assert.ok(compact.slice(compact.indexOf('APPENDIX — FULL REPORT')).includes(context), 'options and their needs remain in appendix');
}
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
assert.equal(reportLength(100 - 96.825), '3.175');
assert.equal(reportLength(-8.825), '-8.83');
assert.equal(reportLength(16.825), '16.83');
assert.equal(reportDuration((22 * 60 + 38) * 60 + 16), '22 h 38 min');
assert.equal(sheetWords('Closed_slot with Radiused_slot_end_type ends; bottom_and_side_finish_milling; Vee_profile; full_retract; unknown_identifier'), 'slot with rounded ends; finish bottom and sides; V-channel; full-retract pecks; unknown identifier');
assert.equal(sheetWords('Check cut {"detail":{"bad":[1,2]}} before cutting'), 'Check cut detail: bad: 1, 2 before cutting');
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
assert.equal(reportDocument(nearMinute, options).blocks.find(b => b.id === 'operations').rows[0][7], '1 min 0 s', 'operation time now uses min/s below one hour, carrying rounded seconds');
// Review fixes: preserve warning content, small lengths and unrelated numbers.
assert.equal(operationTime(17 * 3600 + 9 * 60 + 12), '17 h 9 min');
assert.equal(operationTime(28 * 60 + 56), '28 min 56 s');
assert.equal(operationTime(17 * 3600 + 9 * 60 + 59), '17 h 10 min');
assert.equal(operationTime(3599.6), '1 h 0 min');
assert.equal(reportLength(0.000123456), '0.000123');
assert.equal(reportLength(-0.0123456), '-0.0123');
assert.equal(sheetWords('ratio 0.123456; value 1.234567; length 11.8364 mm; tiny 0.000123456 mm'), 'ratio 0.123456; value 1.234567; length 11.84 mm; tiny 0.000123 mm');
const gripWarning = sheetWords('grip band: [{"operation":"top-2-roughing","faces":["stock.y.min"]},{"operation":"top-5-holes"}]');
assert.equal(sheetWords('offset {"offset_mm":0.000123456,"fraction":0.123456}'), 'offset offset mm: 0.000123; fraction: 0.123456');
assert.equal(gripWarning, 'grip band: top-2-roughing (faces: stock.y.min), top-5-holes');
assert.ok(workholdingText({ available: true, finished: true, setups: 1, extra_stock_mm: 0 }).summary.includes('0:00'), 'page retains main’s missing-time display');
assert.ok(setupZero({ stock: data.plan.setups[0].stock }).includes("zero on the part's box"), 'page retains main’s missing-zero wording');
const clamped = { id: 'top', zero: 'stock', stock: { min: [0,0,-10], max: [20,30,0] }, clamping: { faces: ['stock.y.min','stock.y.max'], jaw_top_z: -8, grip_height_mm: 2, parallels: [{ min: [0,0,-20], max: [20,3,-10] }] } };
const details = setupDetails(clamped, { part: {} }).join(' ');
for (const phrase of ['stock −Y face', 'stock +Y face', 'Jaw-top Z: -8 mm', 'Grip depth: 2 mm', 'height 10 mm, width 3 mm', 'minimum-X/minimum-Y corner', 'top of the stock']) assert.ok(details.includes(phrase), phrase);
assert.ok(setupDetails({}, {}).join(' ').includes('not stated in the plan'));
const namedMachine = reportDocument(data, { ...options, library: { machines: [{ id: 'mill', name: 'the Makera Carvera' }] } });
assert.equal(namedMachine.blocks.find(b => b.id === 'header').rows[0][0], 'Makera Carvera');
const refusedThread = structuredClone(data);
refusedThread.analysis.features[0].faces = [48,57];
refusedThread.analysis.resume = { answers: { 'hole.function:hole_patterns.set:48.49.57.58': 'tapped:M3x0.5' } };
refusedThread.plan.setups[0].operations[0] = { kind: 'holes', tool: 'a', diameter: 4, bottom: 'through', features: ['hole'] };
refusedThread.programs[0].moves[0].op = 'top-1-holes';
refusedThread.plan.decisions.push(
  { id: 'removed', stage: 'execution', rule: 'execution.refused@1', outcome: 'removed', source: 'rule', subject: 'top-2-thread', operation: { setup: 'top', number: 2, kind: 'thread' }, says: 'Thread cutter cannot enter' },
  { id: 'thread-process', stage: 'process', rule: 'process.thread@1', source: 'rule', subject: 'top:thread', features: ['hole'], superseded_by: 'removed', says: 'Thread planned' },
  { id: 'thread-skip', stage: 'execution', rule: 'execution.refused@1', source: 'rule', subject: 'hole', says: 'Thread cutter cannot enter' },
  { id: 'thread-warning', stage: 'execution', rule: 'execution.refused@1', source: 'rule', subject: 'top-2-thread', outcome: 'warning', features: ['hole'], says: 'Thread cutter cannot enter' },
);
refusedThread.plan.skipped = [{ feature: 'hole', decision: 'thread-skip', reason: 'Thread cutter cannot enter' }];
const refusedDoc = reportDocument(refusedThread, options);
const refusedRow = refusedDoc.blocks.find(b => b.id === 'not-cut').rows[0];
assert.equal(refusedRow[0], 'M3×0.5 thread (top-2-thread) — not cut (hole is drilled)');
assert.ok(!refusedRow[0].includes('⌀4 hole'), 'drilled hole is not labelled uncut');
const refusedSheet = reportText({ blocks: refusedDoc.blocks.slice(0, refusedDoc.appendixStart) });
assert.equal((refusedSheet.match(/Refused by camlib/g) || []).length, 1);
assert.ok(!refusedSheet.includes('operations refused'));
// The refused operation's kind is read from its parts, not from its id: a split spot keeps its part.
const refusedSpot = structuredClone(refusedThread);
Object.assign(refusedSpot.plan.decisions.find(d => d.id === 'removed'), { subject: 'top-3-spot_drill-2', operation: { setup: 'top', number: 3, kind: 'spot_drill', part: 2 } });
refusedSpot.plan.decisions.find(d => d.id === 'thread-warning').subject = 'top-3-spot_drill-2';
assert.equal(reportDocument(refusedSpot, options).blocks.find(b => b.id === 'not-cut').rows[0][0], 'spot drill-2 (top-3-spot drill-2) — not cut');
// Uncertain motions come from the program's count; a run that stopped has only its log.
const uncertain = structuredClone(data);
uncertain.programs[0].uncertain_motions = 3;
uncertain.log = ['note: setup top: per-motion gouge proof: 1 proven, 0 unproven; 0 possible gouge cells decided at 0.01 mm tolerance'];
const checkRow = (d) => reportDocument(d, options).blocks.find(b => b.id === 'checks').rows[0][2];
assert.equal(checkRow(uncertain), "note: setup top: camlib's stock-contact check could not clear 3 motions\n" + uncertain.log[0]);
uncertain.programs[0].uncertain_motions = 0;
assert.equal(checkRow(uncertain), uncertain.log[0]);
const stopped = structuredClone(uncertain);
stopped.error = 'setup bottom: camlib refused it';
stopped.log.unshift("note: setup top: camlib's stock-contact check could not clear 2 motions");
assert.equal(checkRow(stopped), stopped.log.join('\n'));
const repeated = structuredClone(data);
repeated.analysis.features = Array.from({ length: 6 }, (_, n) => ({ id: `edge-${n}`, family: 'fillets', record: { radius: 0.8, side: 'convex' } }));
repeated.plan.skipped = repeated.analysis.features.map(f => ({ feature: f.id, reason: 'not reached from this setup' }));
assert.equal(reportDocument(repeated, options).blocks.find(b => b.id === 'not-cut').rows[0][0], '6 × r0.8 edge round');
const noise = structuredClone(detailed);
noise.plan.setups[0].operations[0].iso14649 = { note: 'ISO rationale retained only in appendix' };
noise.plan.decisions.push({ id: 'peck', stage: 'process', rule: 'process.peck@1', outcome: 'warning', says: 'Peck warning', depth_diameter_ratio: 6.1 });
noise.plan.setups[0].operations[0].because.push('peck');
const noiseReport = reportDocument(noise, options), firstOperation = noiseReport.blocks.find(b => b.id === 'operations').rows[0][1];
assert.equal((firstOperation.match(/full-retract pecks/g) || []).length, 1);
assert.ok(!firstOperation.includes('ISO rationale'));
assert.ok(reportText({ blocks: noiseReport.blocks.slice(noiseReport.appendixStart) }).includes('ISO rationale retained only in appendix'));
assert.ok(document.blocks.find(b => b.id === 'operations').headers.includes('Stepdown / stepover mm'));
const slotShape = structuredClone(data);
slotShape.plan.setups[0].operations[0] = { kind: 'slot', width: 4, length: 8, tool: 'a', iso14649: { note: 'a Closed_slot with Radiused_slot_end_type ends' } };
assert.ok(reportDocument(slotShape, options).blocks.find(b => b.id === 'operations').rows[0][1].includes('slot with rounded ends'));
const onlyOptionNeeds = structuredClone(detailed);
onlyOptionNeeds.plan.tools_needed = [];
const onlyOptionReport = reportDocument(onlyOptionNeeds, compactOptions);
assert.ok(!onlyOptionReport.blocks.slice(0, onlyOptionReport.appendixStart).some(b => b.text === 'Tools needed but not in the library'));
assert.ok(reportText({ blocks: onlyOptionReport.blocks.slice(onlyOptionReport.appendixStart) }).includes('special drill'));
const markup = readFileSync(new URL('../demo/index.html', import.meta.url), 'utf8');
assert.match(markup, /<button(?=[^>]*id="setup-sheet")(?=[^>]*\bdisabled\b)[^>]*>/, 'setup-sheet control starts disabled without a plan');
console.log('Report model, display formatting, compact appendix, feature filtering/notes, escaping, missing fields, ordering, collet changes and repeatability passed.');

// The manual transition and actual stock premise use the existing setup sheet content.
const carried = structuredClone(data);
carried.plan.setups[0].stock_note = 'Starting stock carried from the previous setup (approximate).';
carried.plan.setups[0].before_setup = {kind: 'break_out', says: 'Break out before reclamping; clean the 0.5 mm witness at the bottom perimeter.'};
const carriedText = reportText(reportDocument(carried, options));
assert.ok(carriedText.includes(carried.plan.setups[0].stock_note));
assert.ok(carriedText.includes(carried.plan.setups[0].before_setup.says));
assert.ok(carriedText.indexOf('Break out before') < carriedText.indexOf('Faces in jaws'));
const fallback = structuredClone(data);
fallback.plan.setups[0].stock_note = 'Assumes a freed, part-sized block; stock carry unavailable: closed mesh refused.';
assert.ok(reportText(reportDocument(fallback, options)).includes(fallback.plan.setups[0].stock_note));

// Whether another candidate qualifies is read from its `qualifies` field, not from its words.
const choice = (considered) => toolChoiceRows({ tool: 'a', because: ['t'] }, new Map([['t', { id: 't', stage: 'tools', chosen: 'a', says: 'a cuts it', considered }]])).get('a').reason;
assert.equal(choice([{ candidate: 'b', rejected: 'qualifies, but supplied choice is used', qualifies: true }]), 'a cuts it');
assert.equal(choice([{ candidate: 'b', rejected: 'too wide for the 4 mm width' }]), 'the only one that qualifies');
// Where the words and the field disagree, the field decides.
assert.equal(choice([{ candidate: 'b', rejected: 'qualifies, but supplied choice is used' }]), 'the only one that qualifies');
assert.equal(choice([{ candidate: 'b', rejected: 'held back for a later setup', qualifies: true }]), 'a cuts it');
