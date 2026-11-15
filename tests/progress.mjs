// Exercise the page's real request/progress handlers without a browser or renderer.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const elements = new Map();
const $ = (id) => {
  if (!elements.has(id)) elements.set(id, { hidden: false, textContent: "", children: [] });
  return elements.get(id);
};
$("busy-stages").children = Array.from({ length: 4 }, () => ({}));
const timers = new Set();
const worker = { postMessage() {} };
let shown = 0;
const context = vm.createContext({ $, worker, performance: { now: () => 0 },
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
send(1, { stage: "option", label: "Vise, flip and face" });
assert.equal($("busy-title").textContent, "Planning workholding option: Vise, flip and face…");
send(1, { stage: "plan", setups: 2 });
send(1, { stage: "checking", index: 1, total: 9, label: "Roughing" });
assert.equal($("busy-bar").value, 0);
assert.equal($("busy-bar").max, 9);
assert.equal($("busy-detail").textContent, "Vise, flip and face · Roughing");
run();
assert.equal(timers.size, 1);
send(1, { stage: "checking", index: 9, total: 9 });
assert.equal($("busy-title").textContent, "Loading engines…");
worker.onmessage({ data: { id: 1, data: {} } });
assert.equal($("busy").hidden, false);
assert.equal(shown, 0);
worker.onmessage({ data: { id: 2, data: {} } });
assert.equal($("busy").hidden, true);
assert.equal(timers.size, 0);
assert.equal(shown, 1);
run();
worker.onerror({ message: "test" });
assert.equal($("busy").hidden, true);
assert.equal(timers.size, 0);
console.log("Progress stages, honest counts, replans, stale requests and timer cleanup passed.");
