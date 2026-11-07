// The Rust tests read frozen specify-core output. This checks that output is still what the
// specify-core release the site ships writes for these parts, so the frozen fixtures cannot drift
// away from the real link.
//
//   node tests/specify-link.mjs <directory of the unpacked specify-core wasm package>
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const pkg = path.resolve(process.argv[2]);
const { default: init, IntentEngine } = await import(pathToFileURL(path.join(pkg, "specify_core_wasm.js")));
await init({ module_or_path: fs.readFileSync(path.join(pkg, "specify_core_wasm_bg.wasm")) });

const fixtures = new URL("./fixtures/", import.meta.url);
const read = (name) => fs.readFileSync(new URL(name, fixtures));

// The mesh's triangles, whichever order and corner they come in.
function triangles({ positions, indices }) {
  const corner = (i) => [0, 1, 2].map((k) => positions[3 * i + k].toFixed(3)).join(" ");
  const out = [];
  for (let i = 0; i < indices.length; i += 3) {
    out.push([0, 1, 2].map((k) => corner(indices[i + k])).sort().join("|"));
  }
  return out.sort();
}

// Native and WebAssembly builds differ in the last bits of a float; nothing else may differ.
const NOISE = 1e-6;
function differences(actual, expected, at = "") {
  if (typeof actual === "number" && typeof expected === "number") {
    return Math.abs(actual - expected) <= NOISE ? [] : [`${at}: ${actual} != ${expected}`];
  }
  if (actual === null || expected === null || typeof actual !== "object" || typeof expected !== "object") {
    return actual === expected ? [] : [`${at}: ${JSON.stringify(actual)?.slice(0, 80)} != ${JSON.stringify(expected)?.slice(0, 80)}`];
  }
  const keys = new Set([...Object.keys(actual), ...Object.keys(expected)]);
  return [...keys].flatMap((key) => differences(actual[key], expected[key], `${at}.${key}`));
}

for (const [step, stem] of [
  ["plate.step", "plate"],
  ["synthetic/stepped-plate.step", "synthetic/stepped-plate"],
  ["synthetic/stepped-plate-chamfered.step", "synthetic/stepped-plate-chamfered"],
  ["synthetic/walls/overhang.step", "synthetic/walls/overhang"],
  ["synthetic/walls/island.step", "synthetic/walls/island"],
  ["synthetic/walls/channel.step", "synthetic/walls/channel"],
  ["synthetic/walls/round-boss.step", "synthetic/walls/round-boss"],
]) {
  const name = path.basename(step);
  const bytes = new Uint8Array(read(step));
  const [part] = JSON.parse(IntentEngine.parts(name, bytes));
  const engine = IntentEngine.openPart(name, bytes, part.part);
  const { analysis, mesh } = JSON.parse(engine.snapshot());
  engine.free();
  const found = [
    ...differences(analysis, JSON.parse(read(`${stem}.analysis.json`)), "analysis"),
    ...differences(triangles(mesh), triangles(JSON.parse(read(`${stem}.mesh.json`))), "mesh"),
  ];
  assert.equal(found.length, 0, `${step}: ${found.length} differences:\n${found.slice(0, 20).join("\n")}`);
  console.log(`${step}: the frozen analysis and mesh are what specify-core writes`);
}

assert.throws(() => IntentEngine.parts("nonsense.step", new TextEncoder().encode("not a STEP file")));
console.log("an unreadable file is an error");
