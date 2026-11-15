// Exercise the page's real request/progress handlers without a browser or renderer.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const elements = new Map();
const $ = (id) => {
  if (!elements.has(id)) elements.set(id, { hidden: false, textContent: "", children: [],
    replaceChildren(...rows) { this.children = rows; } });
  return elements.get(id);
};
$("busy-stages").children = Array.from({ length: 4 }, () => ({}));
const timers = new Set();
const worker = { postMessage() {} };
let shown = 0;
const context = vm.createContext({ $, worker, document: { createElement: () => ({}) }, performance: { now: () => 0 },
  setInterval: (fn) => { timers.add(fn); return fn; }, clearInterval: (fn) => timers.delete(fn),
  job: 0, openPart: null, machine: { machine: "test", file: "tools.json" },
  showBanner() {}, countOwnFile() {}, show() { shown++; }, console });
const source = readFileSync(new URL("../demo/app.js", import.meta.url), "utf8");
vm.runInContext(source.slice(source.indexOf("// Progress belongs"), source.indexOf("async function openFile")), context);
const run = () => vm.runInContext('run("plate.step", new ArrayBuffer(0), false)', context);
const send = (id, progress) => worker.onmessage({ data: { id, progress } });
run();
assert.equal(timers.size, 1);
assert.equal($("busy-bar").hidden, true);
send(1, { stage: "recognising" });
assert.equal($("busy-stages").children[0].className, "done");
send(1, { stage: "features", count: 41 });
assert.match($("busy-stages").children[1].textContent, /41 found/);
send(1, { stage: "plan", setups: 3 });
assert.match($("busy-stages").children[2].textContent, /3 setups/);
send(1, { stage: "checking", index: 7, total: 23, label: "Facing (top)" });
assert.equal($("busy-bar").value, 6);
assert.equal($("busy-bar").max, 23);
assert.equal($("busy-detail").textContent, "Facing (top)");
assert.equal($("busy-bar").hidden, false);
send(1, { stage: "replanning", label: "Facing (top)" });
assert.equal($("busy-title").textContent, "Replanning without Facing (top)…");
assert.equal($("busy-bar").hidden, true);
assert.doesNotMatch($("busy-stages").children[2].textContent, /3 setups/);
send(1, { stage: "operation_done", index: 7, total: 23 });
assert.equal($("busy-bar").value, 7);
send(1, { stage: "options", labels: ["Vise, ordinary grip", "Vise, flip and face"] });
send(1, { stage: "option", index: 1, total: 2, label: "Vise, ordinary grip" });
send(1, { stage: "plan", setups: 1 });
send(1, { stage: "checking", index: 17, total: 17, label: "Last cut" });
assert.equal($("busy-bar").value, 16); // last operation still running
send(1, { stage: "operation_done", index: 17, total: 17 });
send(1, { stage: "summarising" });
assert.equal($("busy-title").textContent, "Preparing checked toolpaths…");
assert.equal($("busy-bar").hidden, true);
send(1, { stage: "option_result", index: 1, available: true });
send(1, { stage: "option", index: 2, total: 2, label: "Vise, flip and face" });
assert.equal($("busy-stages").children.length, 6);
assert.equal($("busy-stages").children[3].className, "done");
assert.match($("busy-stages").children[3].textContent, /17 operations/);
assert.equal($("busy-title").textContent, "Planning workholding option 2 of 2: Vise, flip and face…");
send(1, { stage: "plan", setups: 2 });
send(1, { stage: "checking", index: 3, total: 18, label: "Roughing" });
assert.equal($("busy-bar").value, 2);
assert.equal($("busy-bar").max, 18);
assert.equal($("busy-title").textContent, "Checking workholding option 2 of 2: Vise, flip and face");
assert.equal($("busy-detail").textContent, "Vise, flip and face · Roughing");
send(1, { stage: "operation_done", index: 3, total: 18 });
assert.match($("busy-stages").children[4].textContent, /3 \/ 18/);
send(1, { stage: "comparing" });
assert.equal($("busy-title").textContent, "Comparing the finished stock with the model…");
assert.equal($("busy-bar").hidden, true);
send(1, { stage: "replanning", title: "Trying another seating for the flip" });
assert.equal($("busy-stages").children[4].className, "current");
send(1, { stage: "plan", setups: 2 });
assert.doesNotMatch($("busy-stages").children[4].textContent, /3 \/ 18/);
send(1, { stage: "option_result", index: 2, available: false, reason: "flip unavailable" });
assert.match($("busy-stages").children[4].textContent, /✗ not possible for this part: flip unavailable/);
assert.notEqual($("busy-stages").children[4].className, "done");
send(1, { stage: "choosing" });
assert.equal($("busy-stages").children[5].className, "current");
assert.equal($("busy-title").textContent, "Choosing workholding…");
send(1, { stage: "output" });
assert.equal($("busy-title").textContent, "Preparing results…");
assert.match($("busy-stages").children[5].textContent, /Choosing workholding and locating setup zeros/);
send(1, { stage: "zeros" });
assert.equal($("busy-title").textContent, "Locating setup zeros and positioning toolpaths…");
send(1, { stage: "encoding" });
assert.equal($("busy-title").textContent, "Encoding results…");
// An option rejected before its run still has its own failed row.
run();
send(2, { stage: "options", labels: ["Ordinary", "Flip"] });
send(2, { stage: "option", index: 1, total: 2, label: "Ordinary" });
send(2, { stage: "option_result", index: 1, available: false, reason: "run failed" });
send(2, { stage: "option", index: 2, total: 2, label: "Flip" });
send(2, { stage: "option_result", index: 2, available: false, reason: "no seating" });
send(2, { stage: "choosing" });
assert.match($("busy-stages").children[3].textContent, /✗.*run failed/);
assert.match($("busy-stages").children[4].textContent, /✗.*no seating/);
run();
assert.equal(timers.size, 1);
send(1, { stage: "checking", index: 9, total: 9 });
assert.equal($("busy-title").textContent, "Loading engines…");
assert.equal($("busy-stages").children.length, 5);
assert.equal($("busy-stages").children[3].textContent, "Checking toolpaths");
worker.onmessage({ data: { id: 1, data: {} } });
assert.equal($("busy").hidden, false);
assert.equal(shown, 0);
worker.onmessage({ data: { id: 3, data: {} } });
assert.equal($("busy").hidden, true);
assert.equal(timers.size, 0);
assert.equal(shown, 1);
run();
worker.onerror({ message: "test" });
assert.equal($("busy").hidden, true);
assert.equal(timers.size, 0);
console.log("Progress stages, honest counts, replans, stale requests and timer cleanup passed.");

// Whole-part highlighting follows local tool depth, with rapids and plunges excluded.
const layers = vm.createContext({ Math });
vm.runInContext(source.slice(source.indexOf("function findLayers"), source.indexOf("function nearestFeature")), layers);
vm.runInContext(`
const segments = [{op:"rough"}, {op:"rough",rapid:true}, {op:"rough"},
  {op:"rough"}, {op:"rough"}, {op:"rough"}, {op:"feature"}, {op:"feature"}];
const local = [0,0,-1, 5,0,-1, 5,0,-1, 0,1,-1, 0,1,-1, 5,1,-1.00001,
  5,1,-1, 5,1,-2, 5,1,-2, 0,1,-2, 0,1,-2, 5,2,-2, 0,0,-2, 5,0,-2, 0,0,-3, 5,0,-3];
for (const s of segments) s.kind = s.rapid ? "rapid" : "cut";
findLayers(segments, local);
`, layers);
const segments = vm.runInContext("segments", layers);
assert.equal(segments[0].layer, segments[2].layer);
assert.equal(segments[0].layer.start, 0);
assert.equal(segments[0].layer.end, 2);
assert.equal(segments[1].layer, null);
assert.equal(segments[3].layer, null);
assert.notEqual(segments[2].layer, segments[4].layer);
assert.equal(segments[4].layer, segments[5].layer);
assert.notEqual(segments[5].layer, segments[6].layer);
const colors = () => ({ array: new Float32Array(segments.length * 8) });
const paint = vm.createContext({ model: { segments, operations: new Map([
  ["rough", {color:"orange",features:[]}], ["feature", {color:"blue",features:["hole"]}]
]), futureColors: colors(), doneColors: colors() }, THREE: { Color: class {
  copy() {} set() {} r=1; g=1; b=1;
} }, activeFocus: () => null, currentSegment: () => 4, showCurrent:true, playing:true,
 time:1, paintedOp:null, paintedLayer:null, RAPID:"gray", UNPLANNED:"gray", showRapids:true,
 showStock:true, right:{ block:{}, focus:{} }, drawRight() {} });
vm.runInContext(source.slice(source.indexOf("function paintToolpaths"), source.indexOf("// The feature list, the list of moves")), paint);
vm.runInContext("paintToolpaths()", paint);
const alpha = (i) => vm.runInContext(`model.futureColors.array[${8*i+3}]`, paint);
assert.ok(Math.abs(alpha(0) - 0.13) < 1e-6);
assert.ok(Math.abs(alpha(3) - 0.13) < 1e-6);
assert.ok(Math.abs(alpha(4) - 0.75) < 1e-6);
assert.ok(Math.abs(alpha(5) - 0.75) < 1e-6);
paint.currentSegment = () => 6;
vm.runInContext("paintToolpaths()", paint);
assert.ok(Math.abs(alpha(6) - 0.75) < 1e-6);
assert.ok(Math.abs(alpha(7) - 0.75) < 1e-6); // feature operations still highlight all depths
console.log("Whole-part depth layers, faint other layers and unchanged feature highlighting passed.");
