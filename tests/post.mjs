// Exercise the actual save buttons and post response handler without a renderer.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
const elements = new Map();
const element = () => ({ children: [], listeners: {}, append(e) { this.children.push(e); },
  replaceChildren() { this.children = []; }, addEventListener(k, fn) { this.listeners[k] = fn; }, click() { this.clicked = true; } });
const $ = (id) => { if (!elements.has(id)) elements.set(id, element()); return elements.get(id); };
const requests = [], downloads = [];
const context = vm.createContext({ $, Blob, URL: { createObjectURL() { return "blob:test"; }, revokeObjectURL() {} },
  document: { createElement(tag) { const e = element(); if (tag === "a") downloads.push(e); return e; } },
  machines: ["haas-vf2", "carvera", "linuxcnc-vmc"].map((machine) => ({ machine, name: machine })),
  worker: { postMessage(m) { requests.push(m); } }, job: 1, setTimeout() {}, count() {}, setPlaying() {} });
const source = readFileSync(new URL("../demo/app.js", import.meta.url), "utf8");
vm.runInContext(source.slice(source.indexOf("const postButtons"), source.indexOf("function drawCode")), context);
const build = (machine) => {
  context.data = { name: "plate.step", library: { machine }, gouge_tolerance_mm: 0.05, programs: [{ setup: "top", job: {} }, { setup: "front", job: {} }] };
  vm.runInContext("buildSaveButtons({ data })", context);
  return $("save").children;
};
let buttons = build("haas-vf2");
assert.equal(buttons.length, 4);
assert.equal(buttons[1].disabled, false);
assert.match(buttons[1].title, /Setting 33 FANUC.*Setting 15 on.*mirrors off.*no G92 shift.*no rotation\/scaling/);
assert.match(buttons[1].title, /not against a model of the Haas itself/);
assert.match(buttons[1].title, /checked to 0\.05 mm where configured/);
buttons[1].listeners.click();
assert.equal(buttons[1].disabled, true);
assert.equal(requests.length, 1);
assert.equal(requests[0].setup, "top");
assert.match($("status").textContent, /Writing Haas G-code for the top setup/);
const refusal = (unproven) => ({ released: false, program: null, accepted: [],
  refused: { statuses: ["uncertain"], message: "export refused: material gouge possible_excess" },
  findings: [{ status: "uncertain", completed: 29345, findings: [{ algorithm: "surface_material_v1",
    target_id: "step2cam-top-part", gouge: { assessment: "possible_excess", definite_cells: 0 },
    sweep_proof: { unproven_moves: unproven, tolerance_mm: 0.05 } }] }], premises: ["verify G54"] });
context.result = refusal(14);
vm.runInContext('postedSetup("top", result)', context);
assert.equal(buttons[1].disabled, true);
assert.equal(buttons[1].title, "camlib could not prove 14 moves stay clear of the part (checked to 0.05 mm), so it refused to write this program.");
assert.equal($("status").textContent, buttons[1].title);
context.result = refusal(128);
vm.runInContext('postedSetup("top", result)', context);
assert.equal(buttons[1].disabled, true);
assert.equal(buttons[1].title, "camlib could not prove 128 moves stay clear of the part (checked to 0.05 mm), so it refused to write this program.");
assert.equal($("status").textContent, buttons[1].title);
assert.ok(!buttons[1].title.includes("material gouge possible_excess"));
context.result = refusal(1);
vm.runInContext('postedSetup("top", result)', context);
assert.match($("status").textContent, /1 move stays clear/);
context.result = { released: false, refused: { statuses: ["uncertain"] }, findings: [{status: "uncertain"}] };
vm.runInContext('postedSetup("top", result)', context);
assert.equal($("status").textContent, "camlib could not prove this setup stays clear of the part, so it refused to write this program.");
context.result = { released: false, refused: { statuses: ["violation"], message: "internal words" } };
vm.runInContext('postedSetup("top", result)', context);
assert.equal($("status").textContent, "camlib found a problem with this setup, so it refused to write this program.");
assert.equal(downloads.length, 0);
assert.match($("checks-detail").children[0].textContent, /verify G54/);
vm.runInContext('postedSetup("front", {released:true, program:"camlib program"})', context);
assert.equal(downloads[0].download, "plate-front-haas-vf2.nc");
assert.equal(downloads[0].clicked, true);
assert.match($("status").textContent, /Camlib decoded and checked/);
assert.match($("status").textContent, /checked to 0\.05 mm/);
buttons = build("carvera");
assert.equal(buttons[1].disabled, true);
assert.match(buttons[1].title, /Carvera G-code waits for a measured spindle-to-bed height/);
buttons = build("linuxcnc-vmc");
assert.equal(buttons.length, 2);
assert.ok(buttons.every((b) => !b.textContent.includes("G-code")));
console.log("G-code button states, refusal, premises and download passed");
