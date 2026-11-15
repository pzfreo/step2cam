// Everything happens here, in the visitor's browser: specify-core recognises the features,
// step2cam plans them and camlib makes and checks the toolpaths. Nothing is uploaded.
import initSpecify, { IntentEngine } from "./specify/specify_core_wasm.js";
import initCam, { process, lastPanic } from "./step2cam/step2cam.js";

import { NOT_FEATURES, operationName } from "./report.js";

// demo/build.sh stamps each deploy's URLs with ?v=<hash>; the wasm files carry the same stamp, so a
// cached copy from an earlier deploy is never paired with this one's code.
const stamp = new URL(import.meta.url).search;
const wasm = (path) => ({ module_or_path: new URL(`${path}${stamp}`, import.meta.url) });
const ready = Promise.all([initSpecify(wasm("./specify/specify_core_wasm_bg.wasm")), initCam(wasm("./step2cam/step2cam_bg.wasm"))]);
const libraries = new Map();
function loadLibrary(url) {
  if (!libraries.has(url)) {
    libraries.set(url, fetch(url).then((response) => {
      if (!response.ok) throw new Error(`Could not load ${url}: ${response.status} ${response.statusText}`);
      return response.text();
    }).catch((error) => {
      libraries.delete(url); // A failed fetch can be retried on the next plan.
      throw error;
    }));
  }
  return libraries.get(url);
}

self.onmessage = async ({ data: { id, name, bytes, library, workholding } }) => {
  const progress = (event) => {
    if (event.operation) {
      event.label = operationName(event.operation, { id: event.setup }, { setups: Array(event.setups) });
      delete event.operation;
    }
    self.postMessage({ id, progress: event });
  };
  try {
    progress({ stage: "engines" });
    const [, toolsText] = await Promise.all([ready, loadLibrary(library.file)]);
    progress({ stage: "recognising" });
    const tools = JSON.parse(toolsText);
    if (workholding) {
      tools.decisions = [...(tools.decisions || []).filter((d) => d.rule.split("@")[0] !== "setup.workholding"),
        { rule: "setup.workholding", subject: "part", choice: workholding, source: "user", reason: "chosen in the page Workholding block" }];
    }
    const input = new Uint8Array(bytes);
    // An assembly opens at its largest part.
    const parts = JSON.parse(IntentEngine.parts(name, input));
    const largest = parts.reduce((a, b) => (b.faces > a.faces ? b : a));
    const notice = parts.length > 1 ? `This file has ${parts.length} parts: showing its largest, ${largest.name}.` : null;
    const engine = IntentEngine.openPart(name, input, largest.part);
    let snapshot;
    try {
      snapshot = JSON.parse(engine.snapshot());
    } finally {
      engine.free();
    }
    const { analysis, mesh } = snapshot;
    progress({ stage: "features", count: analysis.features.filter((f) => !NOT_FEATURES.has(f.family)).length });
    const out = JSON.parse(process(JSON.stringify(analysis), JSON.stringify(mesh), JSON.stringify(tools),
      (event) => progress(JSON.parse(event))));
    self.postMessage({
      id,
      data: { name, notice, analysis, mesh, library, tools: tools.tools, rapid_mm_min: tools.rapid_mm_min, ...out },
    });
  } catch (error) {
    // A panic in step2cam traps with no message; it kept the message.
    const panic = error instanceof WebAssembly.RuntimeError ? lastPanic() : null;
    self.postMessage({ id, error: panic ?? String(error?.message ?? error) });
  }
};
