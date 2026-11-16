// Content and escaping checks; no output hashes or dependency on a browser.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { reportDocument, reportLength, reportDuration, sheetWords, checkText, featureName, operationName, operationTime, setupDetails, workholdingText, setupZero, NOT_FEATURES, toolChoiceRows, warningText } from '../demo/report.js';
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
const declaredCheck = checkText([{ setup: 'top', material: { gouge_tolerance_mm: 0.05,
  limits: {height_mm:0.013,sideways_mm:0.26}, gouge: {volume_mm3:[0,0]}, residual: {volume_mm3:[0,1]} } }]);
assert.match(declaredCheck.headline, /checked to 0\.05 mm/);
assert.match(declaredCheck.compact, /checked to 0\.05 mm/);
assert.ok(declaredCheck.items.some((item) => item.includes('declared permitted depth')));
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
// Partly cut V-channel walls and notch corners get their own brief notes in the operation's row.
const partly = structuredClone(detailed);
partly.plan.decisions.push({ id: 'walls', stage: 'tools', rule: 'process.v_channel.wall_stock@1', outcome: 'warning', says: 'Long wall stock warning', wall_stock_mm: 0.4981 },
  { id: 'corners', stage: 'tools', rule: 'process.notch.corner_radius@1', outcome: 'warning', says: 'Long corner warning', radius: 3, model_radius: 0 });
partly.plan.setups[0].operations[0].because.push('walls', 'corners');
const partlyRow = reportDocument(partly, options).blocks.find(b => b.id === 'operations').rows[0][1];
// The sheet shows lengths to two decimals; the brief text itself (also the app's) keeps three.
assert.ok(partlyRow.includes('Walls left up to 0.5 mm thick at the top'), partlyRow);
assert.equal(warningText({ features: new Map() }, partly.plan.decisions.find(d => d.id === 'walls'), true), 'Walls left up to 0.498 mm thick at the top');
assert.ok(partlyRow.includes('Inside corners left r3 (sharp in the model)'), partlyRow);
assert.ok(!partlyRow.includes('Long wall stock warning') && !partlyRow.includes('Long corner warning'), partlyRow);
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

// Standard provenance is shown only on standard sizes; shop sizes keep existing text.
for (const kind of ['standard', 'library']) {
  const sample = structuredClone(data);
  sample.plan.setups[0] = { ...clamped, operations: sample.plan.setups[0].operations,
    stock_size: { kind }, clamping: { ...clamped.clamping, parallels_size: { kind } } };
  const report = reportDocument(sample, options);
  const label = 'standard size, not checked against your shop';
  assert.equal(report.blocks.find(b => b.id === 'header').rows[0][3].includes(label), kind === 'standard');
  assert.equal(setupDetails(sample.plan.setups[0], sample.plan).join(' ').includes(label), kind === 'standard');
}

for (const modelled of [true, false]) {
  const setup = structuredClone(clamped);
  setup.clamping.parallels_size = { kind: 'computed' };
  if (!modelled) setup.clamping.parallels = [];
  const text = setupDetails(setup, { part: {} }).join(' ');
  assert.ok(text.toLowerCase().includes('not a standard size'));
  assert.ok(!text.includes('standard size, not checked against your shop'));
}

const computedStock = structuredClone(data);
computedStock.plan.setups[0].stock_size = { kind: 'computed' };
assert.ok(reportDocument(computedStock, options).blocks.find(b => b.id === 'header').rows[0][3].includes('not a standard size'));

// M10: the sheet reads the pickup record verbatim in the approved two-line form.
const pickup = { ...clamped, zero: { xy: { description: 'centre of ⌀10 bore (cut in setup top)' }, z: { description: 'datum A face (finished in setup top)' } } };
assert.equal(setupDetails(pickup, {}).at(-1), 'Zero   X0 Y0: centre of ⌀10 bore (cut in setup top)\n       Z0: datum A face (finished in setup top)');
assert.ok(!setupDetails(pickup, {}).join(' ').includes('Work zero'));

// The panel retains its established orientation and stock-size wording.
assert.ok(setupZero(pickup).startsWith('Tool comes down −Z;'));
assert.ok(setupZero(pickup).endsWith('stock 20 × 30 × 10 mm.'));
const withPickup = structuredClone(data);
withPickup.plan.setups[0].zero = pickup.zero;
const zeroTable = reportDocument(withPickup, options).blocks.find(b => b.id === 'setup-zero');
assert.deepEqual(zeroTable.headers, []);
assert.deepEqual(zeroTable.rows, [['Zero', 'X0 Y0: centre of ⌀10 bore (cut in setup top)'], ['', 'Z0: datum A face (finished in setup top)']]);
// Both zero rows occupy the same real PDF column; spaces are not alignment.
const { renderReportPdf } = await import('../demo/report-pdf.js');
let pdfTable;
class PdfStub {
  addFileToVFS() {} addFont() {} setFont() {} setProperties() {} setFontSize() {}
  getNumberOfPages() { return 1; } setPage() {} setTextColor() {} setDrawColor() {} line() {} text() {}
  splitTextToSize(s) { return [s]; }
}
renderReportPdf({ title: 'Zero test', blocks: [zeroTable], appendixStart: 1 }, {
  jsPDF: PdfStub, fonts: { regular: '', bold: '' },
  autoTable(doc, opts) { pdfTable = opts; doc.lastAutoTable = { finalY: 40 }; }
});
assert.deepEqual(pdfTable.body, zeroTable.rows);
assert.equal(pdfTable.columnStyles[0].cellWidth, 15);
assert.equal(pdfTable.columnStyles[1].cellWidth, 165);
assert.equal(pdfTable.styles.cellPadding, 0);
assert.equal(pdfTable.theme, 'plain');
assert.equal(pdfTable.showHead, 'never');
assert.equal(pdfTable.pageBreak, 'avoid');

assert.ok(!markup.includes('setup-sheet-html'));
assert.ok(!readFileSync(new URL('../demo/app.js', import.meta.url), 'utf8').includes('setup-sheet-html'));

// Diagram tests are hermetic: committed meshes/libraries, freshly generated plans,
// the pinned PDF library, and the actual font used for print. No user cache reads.
const { buildSetupDiagram } = await import('../demo/setup-diagram.js');
const { renderDiagramPdf } = await import('../demo/report-pdf.js');
const { jsPDF } = await import('jspdf');
const autoTableModule = await import('jspdf-autotable');
const { readdirSync, existsSync } = await import('node:fs');
const { join } = await import('node:path');
const { fileURLToPath } = await import('node:url');
const { execFile } = await import('node:child_process');
const { promisify } = await import('node:util');
const exec = promisify(execFile);
const font = readFileSync(new URL('../node_modules/dejavu-fonts-ttf/ttf/DejaVuSans.ttf', import.meta.url)).toString('base64');
assert.equal(readFileSync(new URL('./fixtures/fonts/LICENSE',import.meta.url),'utf8'),readFileSync(new URL('../node_modules/dejavu-fonts-ttf/LICENSE',import.meta.url),'utf8'), 'the build ships the pinned font licence without needing npm');
const metrics = new jsPDF();
metrics.addFileToVFS('diagram.ttf', font); metrics.addFont('diagram.ttf', 'Diagram', 'normal'); metrics.setFont('Diagram');
const deps = { jsPDF, autoTable: autoTableModule.default.default || autoTableModule.default, fonts: { regular: font, bold: readFileSync(new URL('../node_modules/dejavu-fonts-ttf/ttf/DejaVuSans-Bold.ttf',import.meta.url)).toString('base64') } };
const near = (a, b) => Math.abs(a - b) < 1e-7;
const overlap = (a, b) => a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1];
function segmentHits(a, b, box) {
  if (Math.max(a[0], b[0]) < box[0] || Math.min(a[0], b[0]) > box[2] || Math.max(a[1], b[1]) < box[1] || Math.min(a[1], b[1]) > box[3]) return false;
  const corners = [[box[0],box[1]], [box[2],box[1]], [box[2],box[3]], [box[0],box[3]]];
  const side = p => (b[0]-a[0])*(p[1]-a[1]) - (b[1]-a[1])*(p[0]-a[0]);
  return !corners.every(p => side(p) > 0) && !corners.every(p => side(p) < 0);
}
let labelsChecked = 0, minPt = Infinity, maxPt = 0;
function checkLabels(view, name) {
  const labels = view.shapes.filter(s => s.type === 'text');
  const boxes = labels.map(s => {
    metrics.setFontSize(s.fontPt);
    const width = metrics.getTextWidth(s.text) / view.scale;
    const height = s.fontPt * 0.352778 / view.scale;
    const x = s.x - (s.anchor === 'middle' ? width / 2 : s.anchor === 'end' ? width : 0);
    assert.ok(s.fontPt >= 7, `${name}: small label ${s.text}: ${s.fontPt} pt`);
    assert.ok(near(s.fontSize * view.scale / 0.352778, s.fontPt));
    minPt = Math.min(minPt, s.fontPt); maxPt = Math.max(maxPt, s.fontPt); labelsChecked++;
    return [x, s.y - height * 0.3, x + width, s.y + height * 1.1];
  });
  const lines = view.shapes.filter(s => ['leader','guide','dimension','axis','edge','hidden','pickup'].includes(s.class) && s.type === 'polyline').map(s => s.points);
  for (const s of view.shapes.filter(s => ['stock','stock-body','jaw','jaw-near','parallel','stop'].includes(s.class) && s.type === 'rect')) lines.push([[s.x,s.y],[s.x+s.width,s.y],[s.x+s.width,s.y+s.height],[s.x,s.y+s.height],[s.x,s.y]]);
  for (const s of view.shapes.filter(s => s.type === 'outline')) lines.push(...s.loops);
  for (const leader of view.shapes.filter(s=>s.class==='leader'||s.class==='guide')) {
    const from=leader.points[0];
    for (const jaw of view.shapes.filter(s=>['jaw','jaw-near'].includes(s.class))) {
      const box=[jaw.x,jaw.y,jaw.x+jaw.width,jaw.y+jaw.height];
      if (from[0]>=box[0]-1e-8&&from[0]<=box[2]+1e-8&&from[1]>=box[1]-1e-8&&from[1]<=box[3]+1e-8) continue;
      for(let i=1;i<leader.points.length;i++) assert.ok(!segmentHits(leader.points[i-1],leader.points[i],box),`${name}: leader crosses an unrelated jaw`);
    }
  }
  const routed = view.shapes.filter(s => s.class === 'leader' || s.class === 'guide');
  const turn = (a,b,c) => (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]);
  for(let i=0;i<routed.length;i++) for(let j=i+1;j<routed.length;j++) {
    for(let u=1;u<routed[i].points.length;u++) for(let v=1;v<routed[j].points.length;v++) {
      const [a,b]=routed[i].points.slice(u-1,u+1),[c,d]=routed[j].points.slice(v-1,v+1);
      if(Math.hypot(a[0]-c[0],a[1]-c[1])<1e-8) continue; // shared pickup origin
      const box=[Math.min(c[0],d[0]),Math.min(c[1],d[1]),Math.max(c[0],d[0]),Math.max(c[1],d[1])];
      const crossing=segmentHits(a,b,box)&&turn(a,b,c)*turn(a,b,d)<=1e-8&&turn(c,d,a)*turn(c,d,b)<=1e-8;
      assert.ok(!crossing,`${name}: leaders cross each other`);
    }
  }
  for(let i=0;i<labels.length;i++) if(labels[i].dimension) {
    const dim=labels[i].dimension, line=view.shapes.find(s=>s.class==='dimension'&&s.type==='polyline'&&s.points[0][1]===dim.low&&s.points[1][1]===dim.high&&s.points[0][0]===s.points[1][0]);
    assert.ok(line,`${name}: own dimension line exists`);
    const x=line.points[0][0],mid=(dim.low+dim.high)/2,box=boxes[i];
    const gap=Math.hypot(Math.max(box[0]-x,0,x-box[2]),Math.max(box[1]-mid,0,mid-box[3]))*view.scale;
    assert.ok(gap<=3.01,`${name}: ${labels[i].text} is ${gap} mm from its dimension midpoint`);
  }
  boxes.forEach((box, i) => {
    assert.ok(box[0] >= view.bounds[0] && box[1] >= view.bounds[1] && box[2] <= view.bounds[2] && box[3] <= view.bounds[3], `${name}: clipped ${labels[i].text}`);
    for (let j = i + 1; j < boxes.length; j++) assert.ok(!overlap(box, boxes[j]), `${name}: ${labels[i].text} overlaps ${labels[j].text}`);
    for (const points of lines) for (let j = 1; j < points.length; j++) assert.ok(!segmentHits(points[j-1], points[j], box), `${name}: edge/leader/dimension crosses ${labels[i].text}`);
    assert.ok(!/^[a-z]/.test(labels[i].text)); assert.ok(!/-\d/.test(labels[i].text));
  });
  assert.ok((view.bounds[2]-view.bounds[0])*view.scale <= 180 + 1e-7);
  assert.ok(view.height <= 95);
}
class DiagramPdfStub extends PdfStub {
  constructor() { super(); this.calls = []; }
  setFillColor(...a) { this.calls.push(['fill', ...a]); }
  setDrawColor(...a) { this.calls.push(['stroke', ...a]); }
  setLineWidth(...a) { this.calls.push(['width', ...a]); }
  setLineDashPattern(...a) { this.calls.push(['dash', ...a]); }
  setFontSize(...a) { this.calls.push(['font', ...a]); }
  rect(...a) { this.calls.push(['rect', ...a]); }
  circle(...a) { this.calls.push(['circle', ...a]); }
  lines(...a) { this.calls.push(['path', ...a]); }
  fillStrokeEvenOdd() { this.calls.push(['outline-fill']); }
  line(...a) { this.calls.push(['line', ...a]); }
  text(...a) { this.calls.push(['text', ...a]); }
  addImage() { assert.fail('PDF must stay vector'); }
  html() { assert.fail('PDF must stay vector'); }
  addPage() {} getTextWidth(s) { return s.length; }
}
function checkPdf(view) {
  const pdf = new DiagramPdfStub(); renderDiagramPdf(pdf, view, 15, 30, 180, view.height);
  for (const type of ['rect','circle','text']) assert.equal(pdf.calls.filter(c => c[0] === type).length, view.shapes.filter(s => s.type === type).length);
  assert.equal(pdf.calls.filter(c => c[0] === 'outline-fill').length, view.shapes.filter(s => s.type === 'outline').length);
  const paths = view.shapes.reduce((n,s) => n + (s.type === 'outline' ? s.loops.length : ['polyline','polygon'].includes(s.type) ? 1 : 0), 0);
  assert.equal(pdf.calls.filter(c => c[0] === 'path').length, paths);
  assert.deepEqual(pdf.calls.filter(c => c[0] === 'text').map(c => c[1]), view.shapes.filter(s => s.type === 'text').map(s => s.text));
  assert.ok(pdf.calls.filter(c => c[0] === 'font').every(c => c[1] >= 7));
  assert.ok(pdf.calls.filter(c => c[0] === 'width').every(c => [0.1,0.15,0.25].includes(c[1])), 'strokes have page-mm widths');
  const rects = view.shapes.filter(s => s.type === 'rect'), calls = pdf.calls.filter(c => c[0] === 'rect');
  rects.forEach((s,i) => {
    const want = [15+(s.x-view.bounds[0])*view.scale, 30+(view.bounds[3]-s.y-s.height)*view.scale, s.width*view.scale, s.height*view.scale];
    assert.ok(want.every((n,k) => near(n,calls[i][k+1])));
    if (s.class === 'stop' || s.class === 'jaw-near') assert.equal(calls[i].at(-1), 'S');
  });
}
const cube = { positions: [0,0,0,10,0,0,10,10,0,0,10,0,0,0,10,10,0,10,10,10,10,0,10,10],
  indices: [0,2,1,0,3,2,4,5,6,4,6,7,0,1,5,0,5,4,1,2,6,1,6,5,2,3,7,2,7,6,3,0,4,3,4,7],
  faces: [[0,2],[2,2],[4,2],[6,2],[8,2],[10,2]], edges: [[2,2,0,8,2,0],[2,2,10,8,2,10]] };
const fixtureSetup = { id: 'top', rotation: [[1,0,0],[0,1,0],[0,0,1]], origin_model: [0,0,10], stock: {min:[0,0,-10],max:[10,10,2]}, operations: [{kind:'facing',floor_z:0}],
  zero: 'stock', clamping: {fixed_jaw:'x.min', grip_height_mm:3,jaw_top_z:-7,jaws:[{min:[-5,-2,-20],max:[0,12,-7]},{min:[10,-2,-20],max:[15,12,-7]}],parallels:[{min:[0,0,-20],max:[2,10,-10]}]} };
const diagram = buildSetupDiagram({mesh:cube}, fixtureSetup, reportLength);
assert.equal(diagram.type, 'setup-diagram', diagram.text);
assert.equal(diagram.warnings.length, 0, 'legacy stock zero agrees with the Zero line');
assert.ok(diagram.views.every(v => v.shapes.some(s => s.class === 'zero')));
assert.ok(diagram.views[0].shapes.some(s => s.class === 'hidden' && s.points.some(p => p[0] === 2 && p[1] === 2)));
assert.ok(diagram.views[0].shapes.some(s => s.class === 'edge' && s.points.some(p => p[0] === 2 && p[1] === 2)));
for (const view of diagram.views) { checkLabels(view, 'cube'); checkPdf(view); }
for (const zero of [undefined, {status:'warning',xy:{description:'very long XY pickup description'},z:{description:'another long Z description'}}]) {
  const d = buildSetupDiagram({mesh:cube},{...fixtureSetup,zero},reportLength);
  assert.equal(d.type, 'setup-diagram', d.text); assert.equal(d.warnings.length, 1);
  assert.ok(d.warnings[0].length < 80 && !d.warnings[0].includes('description'));
  assert.ok(d.views.every(v => !v.shapes.some(s => ['zero','pickup','axis'].includes(s.class))));
}
const noFacing = buildSetupDiagram({mesh:cube},{...fixtureSetup,operations:[]},reportLength);
assert.ok(noFacing.views.every(v => !v.shapes.some(s => s.class === 'stock' || s.dimension?.kind === 'Facing')), 'slack without facing does not become facing');
assert.ok(!noFacing.legend.some(([cls]) => cls === 'stock'));
const isolated = structuredClone(data);
isolated.mesh = cube; isolated.plan.setups = [{...fixtureSetup,id:'bad',origin_model:[Infinity,0,0]}, {...fixtureSetup,id:'good'}];
const warningsLogged=[], savedWarn=console.warn;
console.warn=(...args)=>warningsLogged.push(args);
const isolatedReport = reportDocument(isolated, options);
assert.ok(isolatedReport.blocks.some(b=>b.text==='Setup diagram not drawn: the part position is invalid.'));
assert.ok(warningsLogged.some(args=>args[1] instanceof Error&&args[1].message==='non-finite mesh coordinates'));
assert.equal(isolatedReport.blocks.filter(b => b.type === 'setup-diagram').length, 1);
assert.equal(isolatedReport.blocks.filter(b => b.text?.startsWith('Setup diagram not drawn:')).length, 1);
assert.doesNotThrow(() => renderReportPdf(isolatedReport, deps), 'one broken drawing cannot prevent the PDF');
const failPlacement = buildSetupDiagram({mesh:cube},{...fixtureSetup,zero:{status:'located',xy:{description:'W'.repeat(300),features:[]},z:{description:'Stock top',features:[]}}},reportLength);
assert.equal(failPlacement.type,'paragraph');
assert.equal(failPlacement.text,'Setup diagram not drawn: a label could not be placed clearly.');
assert.ok(warningsLogged.some(args=>args[1] instanceof Error&&args[1].message.startsWith('Cannot place setup diagram label:')));
const loggedBefore=warningsLogged.length;
const overBudget=buildSetupDiagram({mesh:{...cube,indices:Array.from({length:300},()=>cube.indices).flat()}},fixtureSetup,reportLength);
assert.equal(overBudget.text,'Setup diagram not drawn: the part is too detailed to draw here.');
assert.equal(warningsLogged.length,loggedBefore,'expected drawing limits are not unexpected bugs');
console.warn=savedWarn;
assert.ok(!failPlacement.text.includes('WWW'), 'unplaceable caption is not repeated as another unplaceable warning');
assert.doesNotThrow(()=>renderReportPdf({title:'Unavailable',blocks:[failPlacement,diagram],appendixStart:2},deps));
const reverseX = buildSetupDiagram({mesh:cube},{...fixtureSetup,clamping:{...fixtureSetup.clamping,fixed_jaw:'x.max'}},reportLength);
assert.equal(reverseX.type,'setup-diagram',reverseX.text);
for(const view of reverseX.views) {
  const fixed=view.shapes.find(s=>s.text==='Fixed jaw'),moving=view.shapes.find(s=>s.text==='Moving jaw');
  assert.ok(fixed.x>moving.x&&fixed.x>=10,'x.max fixed jaw label is on the recorded side');
  for(const jaw of fixtureSetup.clamping.jaws) assert.ok(view.shapes.some(s=>s.class==='jaw'&&s.x===jaw.min[0]&&s.width===jaw.max[0]-jaw.min[0]),'recorded jaws are drawn');
  checkLabels(view,'x.max fixed jaw');
}
const bottomPickup = buildSetupDiagram({mesh:cube,analysis:{faces:[{id:1,kind:'plane',centroid:[5,5,10],direction:[0,0,1]}]}},{...fixtureSetup,zero:{status:'located',xy:{method:'stock_edges',description:'Stock corner',features:[]},z:{description:'bottom face — above the seat on datum A face',features:['faces:1']}}},reportLength);
assert.equal(bottomPickup.type,'setup-diagram',bottomPickup.text);
assert.ok(bottomPickup.views[1].shapes.some(s=>s.text==='Bottom face'));
assert.ok(!bottomPickup.views[1].shapes.some(s=>s.text==='Datum A face'),'Z caption names the target before the seat reference');
const reverseY = buildSetupDiagram({mesh:cube},{...fixtureSetup,clamping:{...fixtureSetup.clamping,fixed_jaw:'y.max',jaws:[{min:[-2,-5,-20],max:[12,0,-7]},{min:[-2,10,-20],max:[12,15,-7]}]}},reportLength);
assert.equal(reverseY.type,'setup-diagram',reverseY.text);
assert.ok(reverseY.views[1].shapes.some(s=>s.text==='Moving jaw (near side)'), 'the plan can make the far jaw fixed');
const { denseMesh, cylinderMesh, cylinderSetup } = await import('./diagram-meshes.mjs');

for(const n of [100,300]) {
  const dense = buildSetupDiagram({mesh:denseMesh(n)},fixtureSetup,reportLength);
  assert.equal(dense.type,'setup-diagram',dense.text);
  for(const view of dense.views) {
    assert.ok(view.shapes.filter(s=>s.type==='outline').length<=1);
    assert.equal(view.note,view.stats.silhouetteTruncated?'Some curved edges omitted':'');
    assert.ok(view.stats.contourVertices<=8000);
    assert.ok(!view.shapes.some(s=>s.type==='polygon'&&s.class==='part'), 'fill does not become per-triangle commands');
    assert.ok(view.shapes.length<2500&&view.stats.silhouetteEdges<=2048, 'restored silhouette commands stay bounded');
    checkLabels(view,`dense ${4*n*n}`);
  }
  const drawnNotes=[];
  function NotePdf(...args) {
    const doc=new jsPDF(...args), text=doc.text;
    doc.text=function(value,...rest) { if(value==='Some curved edges omitted') drawnNotes.push(value); return text.call(this,value,...rest); };
    return doc;
  }
  const pdf=renderReportPdf({title:'Dense projection',blocks:[dense],appendixStart:1},{...deps,jsPDF:NotePdf});
  assert.equal(drawnNotes.length,dense.views.filter(v=>v.stats.silhouetteTruncated).length,'PDF prints the omission note under each affected view');
  assert.ok(pdf.output('arraybuffer').byteLength<=300000, 'dense setup stays below the requested 300 kB budget');
}
const cylinder = buildSetupDiagram({mesh:cylinderMesh()},cylinderSetup,reportLength);
assert.equal(cylinder.type,'setup-diagram',cylinder.text);
for(const view of cylinder.views) {checkLabels(view,'1000-segment cylinder');checkPdf(view);assert.ok(view.stats.rasterTests<4000000&&view.stats.rasterRows<4000000);}
assert.ok(cylinder.views[1].shapes.some(s=>s.source==='silhouette'&&s.points.length>=2));
// A dense projected bin is explicitly uncertain; its edge must stay solid.
const crowdedMesh={positions:[0,0,10,10,0,10,0,10,10,100,100,10,90,100,10,100,90,10],indices:[...Array.from({length:300},()=>[0,1,2]).flat(),3,4,5],faces:[[0,300],[300,1]],edges:[[1,1,0,2,1,0]]};
const crowded=buildSetupDiagram({mesh:crowdedMesh},{...fixtureSetup,zero:undefined,clamping:undefined,operations:[]},reportLength);
assert.equal(crowded.type,'setup-diagram',crowded.text);
assert.ok(crowded.views[0].shapes.some(s=>s.class==='edge'&&s.source==='brep'&&s.points.some(p=>near(p[0],1)&&near(p[1],1))), 'overcrowded bin falls back to solid, not invented hidden edges');
// Setup-frame coordinates, and the work zero itself, are separate invariants.
const translatedCube=structuredClone(cube);
translatedCube.positions=translatedCube.positions.map((v,i)=>v+[100,200,300][i%3]);
translatedCube.edges=translatedCube.edges.map(edge=>edge.map((v,i)=>v+[100,200,300][i%3]));
const shifted=buildSetupDiagram({mesh:translatedCube},{...fixtureSetup,rotation:[[0,1,0],[-1,0,0],[0,0,1]],origin_model:[110,200,310]},reportLength);
assert.equal(shifted.type,'setup-diagram',shifted.text);
assert.deepEqual(shifted.views[0].partBounds,{min:[0,0,-10],max:[10,10,0]});
for(const view of shifted.views) assert.ok(view.shapes.some(s=>s.class==='zero'&&s.x===0&&s.y===0));
const steppedMesh = JSON.parse(readFileSync(new URL('./fixtures/synthetic/stepped-plate.mesh.json',import.meta.url)));
const steppedAnalysis = JSON.parse(readFileSync(new URL('./fixtures/synthetic/stepped-plate.analysis.json',import.meta.url)));
const bore = steppedAnalysis.features.find(f=>f.family==='holes'&&f.record.diameter===10);
const boreView = buildSetupDiagram({mesh:steppedMesh,analysis:steppedAnalysis},{...fixtureSetup,origin_model:[0,0,0],zero:{status:'located',xy:{description:'centre of ⌀10 bore',features:[bore.id]},z:{description:'Stock top',features:[]}}},reportLength);
assert.equal(boreView.type,'setup-diagram',boreView.text);
assert.ok(boreView.views[0].shapes.some(s=>s.text==='⌀10 bore'));
const boreEdges=boreView.views[0].shapes.filter(s=>s.feature===bore.id);
assert.ok(boreEdges.length&&boreEdges.every(s=>s.points.every(p=>Math.abs(Math.hypot(...p)-5)<0.01)), 'pickup follows the real bore boundary');
for (const view of boreView.views) checkLabels(view,'bore pickup');
const walk = dir => readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(join(dir,e.name)):[join(dir,e.name)]);
const fixtureRoot = fileURLToPath(new URL('./fixtures/',import.meta.url));
const meshPaths = walk(fixtureRoot).filter(p=>p.endsWith('.mesh.json'));
const rust = fileURLToPath(new URL('../rust/',import.meta.url));
const built = await exec('cargo',['build','--release','--example','process','--message-format=json'],{cwd:rust,maxBuffer:32*1024*1024});
const executable = built.stdout.trim().split('\n').map(line=>JSON.parse(line)).findLast(m=>m.reason==='compiler-artifact'&&m.target.name==='process'&&m.executable)?.executable;
assert.ok(executable, 'native fixture planner built');
// Frozen schema-v1 plans remain supported as well as today's generated records.
for (const path of walk(join(fixtureRoot,'rust-golden')).filter(p=>p.endsWith('.plan.json'))) {
  const name=path.split('/').at(-1).replace(/(?:-thread)?\.plan\.json$/,''), meshPath=join(fixtureRoot,name+'.mesh.json');
  if (!existsSync(meshPath)) continue;
  const plan=JSON.parse(readFileSync(path)),mesh=JSON.parse(readFileSync(meshPath)),analysis=JSON.parse(readFileSync(meshPath.replace('.mesh.json','.analysis.json')));
  for (const setup of plan.setups) {
    const d=buildSetupDiagram({mesh,analysis,plan},setup,reportLength); assert.equal(d.type,'setup-diagram',d.text);
    for(const view of d.views) {checkLabels(view,`legacy ${name}`);checkPdf(view);}
    if(setup.zero==='stock') assert.equal(d.warnings.length,0);
  }
}
const libraries = ['libraries/haas-vf2.json','demo/tools.json'];
const tasks = meshPaths.flatMap(meshPath=>libraries.map(library=>({meshPath,library})));
let next = 0, setupsChecked = 0;
const nonPlanLabels = labelsChecked;
async function checkRealPlans() {
  while (next < tasks.length) {
    const {meshPath,library} = tasks[next++], analysisPath = meshPath.replace('.mesh.json','.analysis.json');
    const output = await exec(executable,[analysisPath,meshPath,fileURLToPath(new URL('../'+library,import.meta.url))],{maxBuffer:128*1024*1024,timeout:240000});
    const sample = JSON.parse(output.stdout), mesh = JSON.parse(readFileSync(meshPath)), analysis = JSON.parse(readFileSync(analysisPath));
    const tables = [];
    const autoTable = (doc, opts) => {
      const operations = opts.head?.[0]?.[0] === '#', heads = new Set(), bodies = new Set();
      deps.autoTable(doc,{...opts,didDrawCell(cell) {
        if (operations) (cell.section === 'head' ? heads : bodies).add(doc.getCurrentPageInfo().pageNumber);
      }});
      if (operations) tables.push({heads,bodies});
    };
    renderReportPdf(reportDocument({...sample,mesh,analysis},{date:'15/11/2026',library:JSON.parse(readFileSync(fileURLToPath(new URL('../'+library,import.meta.url))))}),{...deps,autoTable});
    for (const table of tables) for (const page of table.heads) assert.ok(table.bodies.has(page),`${meshPath}: operations header has a body row on page ${page}`);
    for (const setup of sample.plan.setups) {
      const name = `${meshPath.split('/').at(-1)} ${library} ${setup.id}`;
      const d = buildSetupDiagram({...sample,mesh,analysis},setup,reportLength);
      assert.equal(d.type,'setup-diagram',`${name}: ${d.text}`); setupsChecked++;
      for (const view of d.views) { checkLabels(view,name); checkPdf(view); assert.ok(view.stats.contourVertices <= 8000); }
      const transformed = Array.from({length:mesh.positions.length/3},(_,i)=>setup.rotation.map(axis=>axis.reduce((n,r,j)=>n+r*(mesh.positions[i*3+j]-setup.origin_model[j]),0)));
      for (let axis=0;axis<3;axis++) {
        assert.ok(near(d.views[0].partBounds.min[axis],Math.min(...transformed.map(p=>p[axis]))));
        assert.ok(near(d.views[0].partBounds.max[axis],Math.max(...transformed.map(p=>p[axis]))));
      }
      if (setup.zero?.status==='located') for (const view of d.views) assert.ok(view.shapes.some(s=>s.class==='zero'&&s.x===0&&s.y===0));
      if (setup.zero?.status==='located') {
        const topLabels=d.views[0].shapes.filter(s=>s.type==='text').map(s=>s.text), frontLabels=d.views[1].shapes.filter(s=>s.type==='text').map(s=>s.text);
        assert.ok(topLabels.includes('X0 Y0')&&frontLabels.includes('Z0 here'));
        for(const pickup of Object.values(setup.zero.xy.axes||{})) {
          const datum=pickup.description.match(/datum [A-Z] face/)?.[0];
          if(datum) assert.ok(topLabels.includes(datum[0].toUpperCase()+datum.slice(1)),`${name}: recorded XY datum is labelled`);
        }
      }
      // Clip a horizontal mesh triangle to each support rectangle. A positive
      // intersection area proves seating; overlapping boxes alone do not.
      function seatArea(triangle, parallel) {
        let poly=triangle.map(p=>p.slice(0,2));
        for(let axis=0;axis<2;axis++) for(const lower of [true,false]) {
          const value=(lower?parallel.min:parallel.max)[axis], out=[];
          for(let i=0;i<poly.length;i++) {
            const a=poly[i],b=poly[(i+1)%poly.length],inside=p=>lower?p[axis]>=value:p[axis]<=value;
            if(inside(a)) out.push(a);
            if(inside(a)!==inside(b)) {const t=(value-a[axis])/(b[axis]-a[axis]);out.push(a.map((v,j)=>v+t*(b[j]-v)));}
          }
          poly=out;
        }
        return Math.abs(poly.reduce((sum,a,i)=>{const b=poly[(i+1)%poly.length];return sum+a[0]*b[1]-a[1]*b[0];},0))/2;
      }
      for(const parallel of setup.clamping?.parallels||[]) {
        const z=parallel.max[2];
        if(setup===sample.plan.setups[0]) assert.ok(near(setup.stock.min[2],z),`${name}: stock seats on parallels`);
        else {
          let area=0;
          for(let i=0;i<mesh.indices.length;i+=3) {
            const tri=mesh.indices.slice(i,i+3).map(v=>transformed[v]);
            if(tri.every(p=>near(p[2],z))) area+=seatArea(tri,parallel);
          }
          assert.ok(area>1e-8,`${name}: mesh seats on parallel tops with nonzero area`);
        }
      }
      if(meshPath.endsWith('/stepped-plate.mesh.json')&&library.includes('haas')&&setup.id==='bottom') {
        function wall(view,face,x,low,high) {
          const segments=view.shapes.filter(s=>s.source==='silhouette'&&s.face===face&&s.class==='hidden').flatMap(s=>s.points.slice(1).map((p,i)=>[s.points[i],p]));
          return segments.some(([a,b])=>Math.abs(a[0]-x)<0.02&&Math.abs(b[0]-x)<0.02&&Math.min(a[1],b[1])<=low+0.02&&Math.max(a[1],b[1])>=high-0.02);
        }
        for(const x of [45,55]) assert.ok(wall(d.views[1],36,x,-18,0),`${name}: hidden bore wall at X${x}`);
        for(const hole of analysis.features.filter(f=>f.family==='holes'&&f.record.diameter===2.5)) {
          const x=hole.record.location[0]-setup.origin_model[0], face=hole.faces.find(f=>analysis.faces.find(a=>a.id===f)?.kind==='cylinder');
          for(const edge of [x-1.25,x+1.25]) assert.ok(wall(d.views[0],face,edge,46,56),`${name}: 10 mm side-hole wall`);
        }
        assert.ok(d.views[0].shapes.some(s=>s.text==='Datum B face')&&d.views[0].shapes.some(s=>s.text==='Datum C face'));
        assert.ok(!d.views[0].shapes.some(s=>s.text==='Datum A face')&&d.views[1].shapes.some(s=>s.text==='Datum A face'));
        assert.ok(!d.views[1].shapes.some(s=>s.text==='Datum C face'));
      }

      if (setup.clamping?.part_stop && setup.zero?.status==='located') for (const view of d.views) {
        const stop = view.shapes.find(s=>s.class==='stop'); assert.ok(stop);
        assert.equal(stop.contact[2],setup.clamping.part_stop.contact_z_mm);
        assert.equal(stop.contact[setup.clamping.part_stop.axis==='x'?0:1],setup.clamping.part_stop.coordinate_mm);
        if (view===d.views[1] && stop.maximum_z_mm!=null) assert.ok(near(stop.y+stop.height,stop.maximum_z_mm));
      }
      if (meshPath.endsWith('/stepped-plate.mesh.json')&&setup.id==='bottom') assert.ok(d.views[0].shapes.some(s=>s.class==='hidden'&&s.points.some(p=>near(p[0],66)&&near(p[1],28))), 'real boss stays hidden below the plate');

      const front = d.views[1], c = setup.clamping, grip = front.shapes.find(s=>s.dimension?.kind==='Grip')?.dimension;
      if (c) {
        assert.ok(grip,`${name}: grip is drawn`);
        assert.ok(near(grip.value,c.grip_height_mm),`${name}: diagram agrees with sheet Grip depth`);
        assert.ok(near(grip.low,c.jaw_top_z-c.grip_height_mm)); assert.equal(grip.high,c.jaw_top_z);
        assert.ok(setupDetails(setup,sample.plan).join(' ').includes(`Grip depth: ${reportLength(grip.value)} mm`));
      }
      const facing = setup.operations.find(o=>o.kind==='facing'), drawn = front.shapes.find(s=>s.dimension?.kind==='Facing');
      assert.equal(Boolean(drawn),Boolean(facing&&setup.stock.max[2]>facing.floor_z),`${name}: facing only for a facing operation`);
      assert.equal(d.views.some(v=>v.shapes.some(s=>s.class==='stock')),Boolean(facing&&setup.stock.max[2]>facing.floor_z));
      if (drawn) { assert.equal(drawn.dimension.low,facing.floor_z); assert.equal(drawn.dimension.high,setup.stock.max[2]); assert.equal(drawn.dimension.value,setup.stock.max[2]-facing.floor_z); }
      if (setup===sample.plan.setups[0]) {
        const body = front.shapes.find(s=>s.class==='stock-body'); assert.ok(body,`${name}: material held is drawn`);
        assert.equal(body.y,setup.stock.min[2]); assert.ok(near(body.y+body.height,setup.stock.max[2]));
      }
      if (c?.fixed_jaw?.startsWith('y')) {
        const nearJaw = c.jaws.reduce((a,b)=>a.min[1]<b.min[1]?a:b);
        const actual = front.shapes.find(s=>s.class==='jaw-near'); assert.ok(actual);
        assert.equal(actual.y,nearJaw.min[2]); assert.equal(actual.x,nearJaw.min[0]);
        assert.ok(front.shapes.findIndex(s=>s.class==='jaw-near')>front.shapes.findIndex(s=>s.class==='part'));
        const expected = c.fixed_jaw==='y.min'?'Fixed jaw (near side)':'Moving jaw (near side)';
        assert.ok(front.shapes.some(s=>s.text===expected));
        assert.ok(!front.shapes.some(s=>s.text==='Fixed jaw'||s.text==='Moving jaw'));
      }
      for (const [cls] of d.legend) assert.ok(d.views.some(v=>v.shapes.some(s=>s.class===cls||(cls==='jaw'&&s.class==='jaw-near'))));
      assert.equal(d.legend.some(([cls])=>cls==='stop'),d.views.some(v=>v.shapes.some(s=>s.class==='stop')));
      assert.ok(!d.legend.some(([,text])=>text==='Hidden below'));
    }
  }
}
await Promise.all([checkRealPlans(),checkRealPlans()]);
assert.equal(tasks.length,24,'all 12 committed meshes on both machines');
assert.ok(setupsChecked>=44);
console.log(`Real fixture diagrams: ${setupsChecked} setups, ${labelsChecked-nonPlanLabels} measured labels, ${minPt}–${maxPt} pt; PDF calls and clearances passed.`);
