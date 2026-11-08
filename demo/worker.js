// Everything happens here, in the visitor's browser: specify-core recognises the features,
// step2cam plans them and camlib makes and checks the toolpaths. Nothing is uploaded.
import initSpecify, { IntentEngine } from "./specify/specify_core_wasm.js";
import initCam, { process } from "./step2cam/step2cam.js";

const ready = Promise.all([initSpecify(), initCam()]);
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

self.onmessage = async ({ data: { id, name, bytes, library } }) => {
  try {
    const [, toolsText] = await Promise.all([ready, loadLibrary(library.file)]);
    const tools = JSON.parse(toolsText);
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
    const out = JSON.parse(process(JSON.stringify(analysis), JSON.stringify(mesh), toolsText));
    self.postMessage({
      id,
      data: { name, notice, analysis, mesh, library, tools: tools.tools, rapid_mm_min: tools.rapid_mm_min, ...out },
    });
  } catch (error) {
    self.postMessage({ id, error: String(error?.message ?? error) });
  }
};
