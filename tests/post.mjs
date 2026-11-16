// Exercise the actual save buttons, run invalidation and worker response handler without a renderer.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
const elements = new Map();
const element = () => ({ children: [], listeners: {}, append(e) { this.children.push(e); },
  replaceChildren(...children) { this.children = children; }, addEventListener(k, fn) { this.listeners[k] = fn; }, click() { this.clicked = true; } });
const $ = (id) => { if (!elements.has(id)) elements.set(id, element()); return elements.get(id); };
const requests = [], downloads = [];
const settings = JSON.parse(readFileSync(new URL("../libraries/haas-vf2.json", import.meta.url))).machines[0].controller_settings;
const context = vm.createContext({ $, Blob, URL: { createObjectURL() { return "blob:test"; }, revokeObjectURL() {} },
  document: { createElement(tag) { const e = element(); if (tag === "a") downloads.push(e); return e; } },
  machines: ["haas-vf2", "carvera", "linuxcnc-vmc"].map((machine) => ({ machine, name: machine })),
  worker: { postMessage(m) { requests.push(m); } }, job: 1, model: null, machine: { machine: "haas-vf2" },
  openPart: null, setTimeout() {}, setInterval() {}, clearInterval() {}, performance: { now: () => 0 },
  count() {}, setPlaying() {}, showBanner() {}, countOwnFile() {}, show() {}, console: { error() {} } });
const source = readFileSync(new URL("../demo/app.js", import.meta.url), "utf8");
vm.runInContext(source.slice(source.indexOf("const postButtons"), source.indexOf("function drawCode")), context);
vm.runInContext(source.slice(source.indexOf("// Progress belongs"), source.indexOf("async function openFile")), context);
const build = (machine = "haas-vf2", name = "plate.step", controller = settings) => {
  context.machine = { machine };
  context.model = { data: { name, library: { machine }, controller_settings: structuredClone(controller), gouge_tolerance_mm: 0.05,
    programs: ["top", "front"].map((setup) => ({ setup, job: { verification: { scene: {}, stock_contact: {}, material: setup === "top" ? [{}] : [] } } })) } };
  $("checks-detail").replaceChildren();
  vm.runInContext("buildSaveButtons(model)", context);
  return $("save").children;
};
const reply = (setup, result, error, id = context.job) => context.worker.onmessage({ data: { id, post: true, setup, result, error } });
const refusal = (unproven) => ({ released: false, program: null, accepted: [],
  refused: { statuses: ["uncertain"], message: "export refused: material gouge possible_excess" },
  findings: [{ status: "uncertain", completed: 29345, findings: [{ algorithm: "surface_material_v1",
    target_id: "step2cam-top-part", gouge: { assessment: "possible_excess", definite_cells: 0 },
    sweep_proof: { unproven_moves: unproven, tolerance_mm: 0.05 } }] }], premises: ["verify G54"] });
let buttons = build();
assert.equal(buttons.length, 4);
assert.equal(buttons[1].disabled, false);
assert.match(buttons[1].title, /Setting 33 FANUC.*Setting 15 on.*mirrors off.*no G92 shift.*no rotation\/scaling/);
assert.match(buttons[1].title, /not against a model of the Haas itself/);
assert.match(buttons[1].title, /checked to 0\.05 mm/);
assert.ok(!buttons[1].title.includes("where configured"));
assert.match(buttons[3].title, /did not compare this setup's result with the part/);
assert.doesNotMatch(buttons[3].title, /material checked/);
for (const n of [14, 128, 1]) {
  buttons = build();
  buttons[1].listeners.click();
  assert.equal(buttons[1].disabled, true);
  assert.equal(requests.at(-1).setup, "top");
  assert.match($("status").textContent, /Writing Haas G-code for the top setup/);
  reply("top", refusal(n));
  assert.equal(buttons[1].disabled, true);
  assert.equal(buttons[1].title, `camlib could not prove ${n} ${n === 1 ? "move stays" : "moves stay"} clear of the part (checked to 0.05 mm), so it refused to write this program.`);
  assert.equal($("status").textContent, buttons[1].title);
  assert.ok(!buttons[1].title.includes("material gouge possible_excess"));
  assert.match($("checks-detail").children[0].textContent, /verify G54/);
}
for (const [result, error, expected] of [
  [{ released: false, refused: { statuses: ["uncertain"] }, findings: [{status: "uncertain"}] }, undefined, "camlib could not prove this setup stays clear of the part, so it refused to write this program."],
  [{ released: false, refused: { statuses: ["violation"], message: "internal words" } }, undefined, "camlib found a problem with this setup, so it refused to write this program."],
  [{ released: false, refused: { statuses: ["invalid_input"], message: "No Haas program slot assigned for setup left." } }, undefined, "step2cam refused G-code: No Haas program slot assigned for setup left."],
  [{ released: false, refusal_source: "camlib", refused: { statuses: ["invalid_input"], message: "bad controller input" } }, undefined, "camlib refused G-code: bad controller input"],
  [undefined, "The plan has changed; open the setup again.", "step2cam could not save G-code: The plan has changed; open the setup again."],
  [undefined, "wasm trap", "step2cam could not save G-code: wasm trap"],
]) {
  buttons = build(); buttons[1].listeners.click(); reply("top", result, error);
  assert.equal(buttons[1].disabled, true);
  assert.equal($("status").textContent, expected);
  assert.equal(buttons[1].title, expected);
}
assert.equal(downloads.length, 0);
const effective = structuredClone(settings);
effective.rigid_tapping_option = false;
const released = { released: true, program: "camlib program", unchecked: ["material", "machine"],
  gouge_tolerance_mm: 0.05, controller_settings: effective,
  premise_overrides: { rigid_tapping_option: { installed: true, declared: false } }, premises: ["verify G54"] };
buttons = build();
$("checks").open = false;
for (let i = 0; i < 2; i++) {
  buttons[3].listeners.click(); reply("front", { ...released, premises: [`verify G54 ${i}`] });
  assert.equal(downloads.at(-1).download, "plate-front-haas-vf2.nc");
  assert.equal(downloads.at(-1).clicked, true);
  assert.match(buttons[3].title, /Camlib decoded and checked/);
  assert.match(buttons[3].title, /did not compare this setup's result with the part/);
  assert.doesNotMatch(buttons[3].title, /material checked|where configured/);
  assert.equal($("status").textContent, buttons[3].title);
  assert.equal($("checks-detail").children.length, 1);
  assert.match($("checks-detail").children[0].textContent, new RegExp(`verify G54 ${i}`));
  assert.equal($("checks").open, false);
}
assert.match($("checks-detail").children[0].textContent, /Rigid tapping option: off.*overrides the installed library value on/);
const changed = structuredClone(settings);
changed.declared_premises.setting_33_coordinate_system.value = "haas";
changed.declared_premises.setting_15_h_and_t_code_agreement.value = false;
buttons = build("haas-vf2", "new.step", changed);
assert.match(buttons[1].title, /Setting 33 HAAS, Setting 15 off/);
buttons[1].listeners.click(); reply("top", { ...released, unchecked: ["machine"], controller_settings: changed });
assert.match(buttons[1].title, /Setting 33 HAAS, Setting 15 off/);
assert.match(buttons[1].title, /material checked to 0\.05 mm/);

// A new run immediately disables focused old buttons; its late response changes nothing.
buttons = build(); buttons[1].listeners.click();
const oldId = context.job, before = downloads.length;
vm.runInContext('run("other.step", new ArrayBuffer(0), false)', context);
assert.equal(buttons[1].disabled, true); assert.equal(buttons[3].disabled, true);
const requestCount = requests.length, status = $("status").textContent;
buttons[3].listeners.click(); assert.equal(requests.length, requestCount);
reply("top", released, undefined, oldId);
assert.equal(downloads.length, before); assert.equal($("status").textContent, status);
assert.equal($("checks-detail").children.length, 0);
// Even if the run errors, the old model cannot be posted using the new run id.
context.worker.onmessage({ data: { id: context.job, error: "cannot process" } });
buttons[1].listeners.click(); assert.equal(requests.length, requestCount);
// Model identity and machine identity are independently checked, not just the run id.
buttons = build(); const oldModel = context.model;
context.model = { ...oldModel }; buttons[1].listeners.click(); assert.equal(requests.length, requestCount);
context.model = oldModel; buttons[1].listeners.click(); context.model = { ...oldModel };
reply("top", released); assert.equal(downloads.length, before);
buttons = build(); context.machine = { machine: "carvera" };
const countBefore = requests.length; buttons[1].listeners.click(); assert.equal(requests.length, countBefore);
context.machine = { machine: "haas-vf2" }; buttons[1].listeners.click(); context.machine = { machine: "carvera" };
reply("top", released); assert.equal(downloads.length, before);
buttons = build("carvera"); const carveraTitle = buttons[1].title;
reply("top", released); assert.equal(buttons[1].title, carveraTitle);
assert.equal(buttons[1].disabled, true);
assert.match(carveraTitle, /Carvera G-code waits for a measured spindle-to-bed height/);
buttons[1].listeners.click(); assert.equal(downloads.length, before);
buttons = build("linuxcnc-vmc");
assert.equal(buttons.length, 2);
assert.ok(buttons.every((b) => !b.textContent.includes("G-code")));
console.log("G-code states, actual premises/overrides, honest checks, attributed refusals, bounded details and stale click/result guards passed");
