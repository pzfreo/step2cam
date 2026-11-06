// The step2cam demo page. Draws what worker.js returns: features on the left in
// three-cad-viewer, the posted G-code on the right in three.js, both in model coordinates and
// one camera, with features, operations, toolpaths and G-code lines linked to each other.

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { LineMaterial } from "three/addons/lines/LineMaterial.js";
import { LineSegments2 } from "three/addons/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/addons/lines/LineSegmentsGeometry.js";
import { Display, Viewer } from "three-cad-viewer";

const $ = (id) => document.getElementById(id);
const FACING = "#a6b0be", ROUGHING = "#e7ebf0", RAPID = "#59626f", BODY = "#8d97a6", UNPLANNED = "#6b7482";
const PALETTE = ["#2dd4bf", "#5aa9ff", "#ffb020", "#f472b6", "#a78bfa", "#a3e635", "#fb7185", "#38bdf8"];
const PLAYBACK_SECONDS = 45;
const ROW_HEIGHT = 20;

let model = null;        // the current part, see buildModel
let focusKey = null;     // what is hovered: a feature id or an operation id
let pinKey = null;       // what was clicked
let time = 0;            // playhead, in estimated machining seconds
let playing = false;

// ------------------------------------------------------------------------------------------
// The data: operations, features and the parsed G-code
// ------------------------------------------------------------------------------------------

function buildModel(data) {
  const operations = new Map(), features = new Map();
  for (const f of data.analysis.features) {
    if (f.family !== "hole_patterns") features.set(f.id, { ...f, operations: [], color: UNPLANNED });
  }
  let next = 0;
  for (const setup of data.plan.setups) {
    setup.operations.forEach((op, n) => {
      const id = `${setup.id}-${n + 1}-${op.kind}`;
      const color = op.kind === "facing" ? FACING : op.kind === "roughing" ? ROUGHING : PALETTE[next++ % PALETTE.length];
      operations.set(id, { ...op, id, setup, color, name: operationName(op, setup, data.plan), time: 0 });
      for (const fid of op.features || []) {
        const feature = features.get(fid);
        if (!feature) continue;
        if (!feature.operations.length) feature.color = color;
        feature.operations.push(id);
      }
    });
  }
  const skipped = new Map(data.plan.skipped.map((s) => [s.feature, s.reason]));
  for (const f of features.values()) f.skipped = skipped.get(f.id);
  const covered = new Set(data.plan.covered_by_roughing);
  for (const f of features.values()) f.covered = covered.has(f.id);

  const path = parseProgram(data, operations);
  const m = { data, operations, features, ...path };
  for (const line of m.lines) {
    if (line.op && line.seconds) operations.get(line.op).time += line.seconds;
  }
  compressTimeline(m);
  return m;
}

// Roughing can be 90% of the machining time, which would make the holes flash past. On the
// timeline and in playback an operation takes the square root of its time; the status line
// still gives the estimate at the real feed rates.
function compressTimeline(m) {
  for (const op of m.operations.values()) op.span = Math.sqrt(op.time);
  let end = 0;
  for (const s of m.segments) {
    const op = m.operations.get(s.op);
    end += s.seconds * (op && op.time ? op.span / op.time : 0);
    s.end = end;
  }
  m.total = end;
}

function operationName(op, setup, plan) {
  const where = plan.setups.length > 1 ? ` (${setup.id})` : "";
  if (op.kind === "facing") return "Facing" + where;
  if (op.kind === "roughing") return "Roughing" + where;
  if (op.kind === "counterbore") return `⌀${op.diameter} counterbore` + where;
  if (op.kind === "thread") return `${op.spec} thread` + where;
  if (op.kind === "slot") return `${op.width} × ${op.length} slot` + where;
  if (op.kind === "wall_finish") return "Finish outside wall" + where;
  if (op.kind === "edge_chamfer") return `${op.width} chamfer on r${op.radius} edges` + where;
  return `⌀${op.diameter} ${op.bottom === "through" ? "through" : op.bottom === "drill_point" ? "drill point" : op.bottom}` + where;
}

// Setup coordinates -> model coordinates: the rotation's inverse is its transpose.
function toModel(setup, p) {
  const r = setup.rotation, o = setup.origin_model;
  return [
    o[0] + r[0][0] * p[0] + r[1][0] * p[1] + r[2][0] * p[2],
    o[1] + r[0][1] * p[0] + r[1][1] * p[1] + r[2][1] * p[2],
    o[2] + r[0][2] * p[0] + r[1][2] * p[1] + r[2][2] * p[2],
  ];
}

const PLANES = { 17: [0, 1, 2], 18: [2, 0, 1], 19: [1, 2, 0] };

// G0 G1 G2 G3 in absolute coordinates with incremental arc centres, which is what step2cam's
// post writes (G90, G91.1). Each G-code line becomes one or more straight segments.
function parseProgram(data, operations) {
  const toolByNumber = new Map(data.tools.map((t) => [t.number, t]));
  const rapidRate = data.machine?.rapid_mm_min || 5000;
  const lines = [], segments = [], positions = [];
  let machining = 0;

  for (const program of data.programs) {
    const setup = data.plan.setups.find((s) => s.id === program.setup);
    let motion = 0, plane = PLANES[17], mode = null, feed = 0, pos = null, tool = null, pending = null;
    const ranges = program.operations;
    for (const text of program.nc.split("\n")) {
      if (!text.trim()) continue;
      const words = {};
      const codes = [];
      for (const [, letter, value] of text.matchAll(/([A-Z])(-?[\d.]+)/g)) {
        if (letter === "G" || letter === "M") codes.push(letter + value);
        else words[letter] = Number(value);
      }
      const line = { text, program: program.setup, n: lines.length, first: segments.length, last: segments.length - 1 };
      lines.push(line);
      for (const code of codes) {
        if (["G0", "G1", "G2", "G3"].includes(code)) mode = Number(code.slice(1));
        if (code === "G17" || code === "G18" || code === "G19") plane = PLANES[code.slice(1)];
        if (code === "M6" && pending != null) tool = toolByNumber.get(pending) || null;
      }
      if (words.T != null) pending = words.T;
      if (words.F != null) feed = words.F;
      const isMotion = codes.some((c) => ["G0", "G1", "G2", "G3"].includes(c));
      const range = ranges.find((r) => motion < r.end && motion >= r.start);
      line.op = range ? range.id : null;
      if (!isMotion) continue;
      motion += 1;

      const target = [words.X ?? pos?.[0] ?? 0, words.Y ?? pos?.[1] ?? 0, words.Z ?? pos?.[2] ?? 0];
      line.feature = range ? nearestFeature(operations.get(range.id), target) : null;
      line.rapid = mode === 0;
      if (pos) {
        const pts = mode === 2 || mode === 3 ? arcPoints(pos, target, words, plane, mode) : [target];
        let from = pos, length = 0;
        const rate = mode === 0 ? rapidRate : feed || rapidRate;
        for (const to of pts) {
          const d = Math.hypot(to[0] - from[0], to[1] - from[1], to[2] - from[2]);
          length += d;
          const seconds = d / rate * 60;
          machining += seconds;
          segments.push({ line: line.n, op: line.op, feature: line.feature, rapid: line.rapid, tool, setup, seconds, machining });
          positions.push(...toModel(setup, from), ...toModel(setup, to));
          from = to;
        }
        line.seconds = length / rate * 60;
        line.last = segments.length - 1;
      }
      pos = target;
    }
  }
  return { lines, segments, positions: new Float32Array(positions), machining };
}

function nearestFeature(op, point) {
  if (op?.kind === "slot") return op.features[0];
  if (op?.kind === "edge_chamfer") return null;
  if (!op?.centers) return null;
  let best = null, bestD = Infinity;
  op.centers.forEach((c, i) => {
    const d = Math.hypot(point[0] - c[0], point[1] - c[1]);
    if (d < bestD) { bestD = d; best = op.features[i]; }
  });
  return bestD <= (op.diameter ?? op.start_diameter) ? best : null;
}

function arcPoints(from, to, words, plane, mode) {
  const [a, b, c] = plane;
  const offsets = [words.I ?? 0, words.J ?? 0, words.K ?? 0];
  const ca = from[a] + offsets[a], cb = from[b] + offsets[b];
  const start = Math.atan2(from[b] - cb, from[a] - ca), end = Math.atan2(to[b] - cb, to[a] - ca);
  const radius = Math.hypot(from[a] - ca, from[b] - cb);
  let sweep = end - start;
  if (mode === 2) { if (sweep >= -1e-9) sweep -= 2 * Math.PI; } else if (sweep <= 1e-9) sweep += 2 * Math.PI;
  const steps = Math.max(4, Math.ceil(Math.abs(sweep) / (Math.PI / 18)));
  const pts = [];
  for (let i = 1; i <= steps; i++) {
    const f = i / steps, angle = start + sweep * f, p = [0, 0, 0];
    p[a] = ca + radius * Math.cos(angle);
    p[b] = cb + radius * Math.sin(angle);
    p[c] = from[c] + (to[c] - from[c]) * f;
    pts.push(p);
  }
  pts[pts.length - 1] = to;
  return pts;
}

// ------------------------------------------------------------------------------------------
// Left: the part in three-cad-viewer, one leaf per feature
// ------------------------------------------------------------------------------------------

const ROOT = "Part";
let cadDisplay = null, viewer = null, syncing = false;
let leafByFeature = new Map(), featureByLeaf = new Map();

const leafName = (id) => id.replace(/[^\w-]/g, "_");

function shapeOfFaces(mesh, faceIds, edges) {
  const vertices = [], normals = [], triangles = [], perFace = [];
  for (const faceId of faceIds) {
    const [start, count] = mesh.faces[faceId] || [0, 0];
    if (!count) continue;
    const remap = new Map();
    const faceNormals = [];
    for (let t = start; t < start + count; t++) {
      const ids = [mesh.indices[3 * t], mesh.indices[3 * t + 1], mesh.indices[3 * t + 2]];
      const [p, q, r] = ids.map((i) => mesh.positions.slice(3 * i, 3 * i + 3));
      const u = [q[0] - p[0], q[1] - p[1], q[2] - p[2]], v = [r[0] - p[0], r[1] - p[1], r[2] - p[2]];
      const n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
      for (const i of ids) {
        if (!remap.has(i)) {
          remap.set(i, vertices.length / 3);
          vertices.push(...mesh.positions.slice(3 * i, 3 * i + 3));
          faceNormals.push([0, 0, 0]);
          normals.push(0, 0, 0);
        }
        const k = remap.get(i), base = vertices.length / 3 - faceNormals.length;
        const sum = faceNormals[k - base];
        sum[0] += n[0]; sum[1] += n[1]; sum[2] += n[2];
        triangles.push(k);
      }
    }
    const base = vertices.length / 3 - faceNormals.length;
    faceNormals.forEach((n, i) => {
      const len = Math.hypot(...n) || 1;
      normals[3 * (base + i)] = n[0] / len;
      normals[3 * (base + i) + 1] = n[1] / len;
      normals[3 * (base + i) + 2] = n[2] / len;
    });
    perFace.push(count);
  }
  return {
    vertices, normals, triangles, edges,
    obj_vertices: [],
    face_types: perFace.map(() => 0),
    edge_types: Array(edges.length / 6).fill(0),
    triangles_per_face: perFace,
    segments_per_edge: Array(edges.length / 6).fill(1),
  };
}

function edgeSegments(mesh) {
  const out = [];
  for (const polyline of mesh.edges || []) {
    for (let i = 0; i + 5 < polyline.length; i += 3) out.push(...polyline.slice(i, i + 6));
  }
  return out;
}

const LOC = [[0, 0, 0], [0, 0, 0, 1]];

function leaf(parent, name, shape, color, hasEdges) {
  return {
    version: 3, id: `${parent}/${name}`, name, type: "shapes", subtype: "solid", shape,
    state: [1, hasEdges ? 1 : 3], color, alpha: 1, texture: null, loc: LOC, renderback: false, accuracy: null, bb: null,
  };
}

function buildShapes(m) {
  const mesh = m.data.mesh;
  leafByFeature = new Map();
  featureByLeaf = new Map();
  const rootId = `/${ROOT}`;
  const used = new Set(), parts = [];
  for (const f of m.features.values()) {
    const faces = (f.faces || []).filter((i) => !used.has(i));
    if (!faces.length) continue;
    faces.forEach((i) => used.add(i));
    const name = leafName(f.id);
    parts.push(leaf(rootId, name, shapeOfFaces(mesh, faces, []), f.color, false));
    leafByFeature.set(f.id, `${rootId}/${name}`);
    featureByLeaf.set(name, f.id);
  }
  const rest = mesh.faces.map((_, i) => i).filter((i) => !used.has(i));
  parts.unshift(leaf(rootId, "Body", shapeOfFaces(mesh, rest, edgeSegments(mesh)), BODY, true));
  const [lo, hi] = mesh.bbox;
  return {
    version: 3, id: rootId, name: ROOT, loc: LOC, normal_len: 0, parts,
    bb: { xmin: lo[0], ymin: lo[1], zmin: lo[2], xmax: hi[0], ymax: hi[1], zmax: hi[2] },
  };
}

function showPart(m) {
  const box = $("cad").parentElement.getBoundingClientRect();
  const size = { cadWidth: Math.floor(box.width), height: Math.floor(box.height), treeWidth: 0 };
  if (!cadDisplay) {
    cadDisplay = new Display($("cad"), {
      ...size, theme: "dark", glass: true, tools: false, measureTools: false, selectTool: false,
      explodeTool: false, zebraTool: false, studioTool: false, pinning: false,
    });
    viewer = new Viewer(cadDisplay, size, onCadChange);
  } else {
    viewer.clear();
  }
  viewer.render(buildShapes(m), {
    ambientIntensity: 1.1, directIntensity: 1.2, metalness: 0.3, roughness: 0.65,
    edgeColor: 0x20262e, defaultOpacity: 0.5, normalLen: 0,
  }, { target: [...centre(m)], up: "Z", ortho: true, control: "orbit", axes: false, axes0: false, grid: [false, false, false] });
  viewer.setTheme("dark");
  fitCadMarker(box.width);
  viewer.presetCamera("iso", fitZoom(m));
}

// three-cad-viewer frames the part; the toolpaths go past it, so both views start zoomed out
// far enough to show them.
function fitZoom(m) {
  const [lo, hi] = m.data.mesh.bbox;
  const c = centre(m), part = Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2;
  let reach = part;
  for (let i = 0; i < m.positions.length; i += 3) {
    reach = Math.max(reach, Math.hypot(m.positions[i] - c[0], m.positions[i + 1] - c[1], m.positions[i + 2] - c[2]));
  }
  return part / reach;
}

function centre(m) {
  const [lo, hi] = m.data.mesh.bbox;
  return [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
}

function onCadChange(change) {
  if (change.position || change.quaternion || change.zoom || change.target) copyCameraFromCad();
}

// The feature under the pointer, from three-cad-viewer's own hover pick.
function featureUnderPointer() {
  const info = viewer?.lastObject?.info;
  const path = info?.solidPath ?? info?.path ?? "";
  for (const part of path.split("/").reverse()) {
    if (featureByLeaf.has(part)) return featureByLeaf.get(part);
  }
  return null;
}

let hoverInCad = false;
function initCadLinks() {
  const host = $("cad");
  let queued = false, downAt = null;
  host.addEventListener("pointermove", (event) => {
    if (event.pointerType === "touch" || event.buttons || queued) return;
    queued = true;
    // The viewer's own hover pick is debounced, so read its answer a moment later.
    hoverInCad = true;
    setTimeout(() => {
      queued = false;
      if (hoverInCad) setFocus(featureUnderPointer());
    }, 80);
  });
  host.addEventListener("pointerleave", () => {
    hoverInCad = false;
    setFocus(null);
    litCad = undefined;  // the viewer keeps its last hover lit until told otherwise
    highlightCad();
  });
  host.addEventListener("pointerdown", (event) => { downAt = [event.clientX, event.clientY]; });
  host.addEventListener("pointerup", (event) => {
    if (!downAt || Math.hypot(event.clientX - downAt[0], event.clientY - downAt[1]) >= 4) return;
    if (event.pointerType !== "touch") {
      setPin(featureUnderPointer());
      return;
    }
    setPin(featureAtTap(event));
  });
}

// A touch has no hover for the viewer to have picked from. The right view holds the same mesh in
// the same camera, so look there, nearest first, as a finger is wider than a small hole.
function featureAtTap(event) {
  const rect = $("cad").getBoundingClientRect();
  const ray = new THREE.Raycaster();
  const faces = model.data.mesh.faces;
  const around = [[0, 0]];
  for (const radius of [2, 4, 6, 8, 10, 12]) {
    for (let i = 0; i < 12; i++) around.push([radius * Math.cos(i * Math.PI / 6), radius * Math.sin(i * Math.PI / 6)]);
  }
  for (const [dx, dy] of around) {
    const x = (event.clientX + dx - rect.left) / rect.width * 2 - 1;
    const y = -(event.clientY + dy - rect.top) / rect.height * 2 + 1;
    ray.setFromCamera(new THREE.Vector2(x, y), right.camera);
    const hit = ray.intersectObject(right.ghost)[0];
    if (!hit) continue;
    const face = faces.findIndex(([start, count]) => hit.faceIndex >= start && hit.faceIndex < start + count);
    const feature = [...model.features.values()].find((f) => f.faces?.includes(face));
    if (feature) return feature.id;
  }
  return null;
}

// The viewer's axes marker has a fixed size, which would cover much of a small view.
function fitCadMarker(width) {
  viewer.rendered.orientationMarker.setVisible(width >= 300);
}

// ------------------------------------------------------------------------------------------
// Right: the toolpaths in three.js, in the same camera
// ------------------------------------------------------------------------------------------

const right = { scene: null, renderer: null, camera: null, controls: null, future: null, done: null, ghost: null, tool: null };

function initRight() {
  const host = $("gcode-view");
  right.renderer = new THREE.WebGLRenderer({ antialias: true });
  right.renderer.setPixelRatio(devicePixelRatio);
  right.renderer.setClearColor(0x12161c);
  host.append(right.renderer.domElement);
  right.scene = new THREE.Scene();
  right.scene.add(new THREE.AmbientLight(0xffffff, 1.4));
  const sun = new THREE.DirectionalLight(0xffffff, 1.2);
  sun.position.set(1, -1, 2);
  right.scene.add(sun);
  right.camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 10000);
  right.camera.up.set(0, 0, 1);
  right.controls = new OrbitControls(right.camera, right.renderer.domElement);
  right.controls.addEventListener("change", () => {
    drawRight();
    copyCameraToCad();
  });
  const pointer = new THREE.Vector2();
  const ray = new THREE.Raycaster();
  let queued = false, downAt = null;
  // Hover finds features only: facing and roughing lines cover the whole view, and would dim
  // everything else wherever the pointer went. A click can also pick those operations.
  const pick = (event, featuresOnly) => {
    if (!right.future || !model) return null;
    const rect = right.renderer.domElement.getBoundingClientRect();
    pointer.set((event.clientX - rect.left) / rect.width * 2 - 1, -(event.clientY - rect.top) / rect.height * 2 + 1);
    const pixels = event.pointerType === "touch" ? 12 : 3;
    ray.params.Line.threshold = pixels * (right.camera.top - right.camera.bottom) / right.camera.zoom / rect.height;
    ray.setFromCamera(pointer, right.camera);
    const hits = ray.intersectObject(right.future)
      .map((hit) => model.segments[Math.floor(hit.index / 2)])
      .filter((segment) => segment && !segment.rapid);
    const feature = hits.find((segment) => segment.feature);
    if (feature) return feature.feature;
    return featuresOnly || !hits.length ? null : hits[0].op;
  };
  host.addEventListener("pointermove", (event) => {
    if (event.buttons || queued) return;
    queued = true;
    requestAnimationFrame(() => { queued = false; setFocus(pick(event, true)); });
  });
  host.addEventListener("pointerleave", () => setFocus(null));
  host.addEventListener("pointerdown", (event) => { downAt = [event.clientX, event.clientY]; });
  host.addEventListener("pointerup", (event) => {
    if (downAt && Math.hypot(event.clientX - downAt[0], event.clientY - downAt[1]) < 4) setPin(pick(event));
  });
}

function drawRight() {
  if (right.renderer) right.renderer.render(right.scene, right.camera);
}

function resizeRight() {
  const box = $("gcode-view").getBoundingClientRect();
  right.renderer.setSize(box.width, box.height);
  right.focus?.material.resolution.set(box.width, box.height);
  copyCameraFromCad();
}

function copyCameraFromCad() {
  if (!viewer?.ready || syncing) return;
  const from = viewer.rendered.camera.getCamera(), to = right.camera;
  const box = $("gcode-view").getBoundingClientRect();
  const half = (from.top - from.bottom) / 2, aspect = box.width / box.height;
  syncing = true;
  to.position.set(from.position.x, from.position.y, from.position.z);
  to.quaternion.set(from.quaternion.x, from.quaternion.y, from.quaternion.z, from.quaternion.w);
  to.left = -half * aspect; to.right = half * aspect; to.top = half; to.bottom = -half;
  to.near = from.near; to.far = from.far; to.zoom = from.zoom;
  to.updateProjectionMatrix();
  to.updateMatrixWorld();
  const target = viewer.getCameraTarget();
  right.controls.target.set(target[0], target[1], target[2]);
  drawRight();
  syncing = false;
}

function copyCameraToCad() {
  if (!viewer?.ready || syncing) return;
  syncing = true;
  const c = right.camera, t = right.controls.target;
  viewer.setCameraTarget([t.x, t.y, t.z], false);
  viewer.setCameraPosition([c.position.x, c.position.y, c.position.z], false, false);
  viewer.setCameraZoom(c.zoom, false);
  syncing = false;
}

function showToolpaths(m) {
  for (const key of ["future", "done", "focus", "ghost", "tool", "edges", "stock"]) {
    if (right[key]) { right.scene.remove(right[key]); right[key] = null; }
  }
  const mesh = m.data.mesh;
  const body = new THREE.BufferGeometry();
  body.setAttribute("position", new THREE.BufferAttribute(new Float32Array(mesh.positions), 3));
  body.setIndex(mesh.indices);
  body.computeVertexNormals();
  right.ghost = new THREE.Mesh(body, new THREE.MeshLambertMaterial({ color: 0x4a5462, transparent: true, opacity: 0.35, depthWrite: false, side: THREE.DoubleSide }));
  right.scene.add(right.ghost);

  const edges = new THREE.BufferGeometry();
  edges.setAttribute("position", new THREE.BufferAttribute(new Float32Array(edgeSegments(mesh)), 3));
  right.edges = new THREE.LineSegments(edges, new THREE.LineBasicMaterial({ color: 0x626d7c }));
  right.scene.add(right.edges);

  const stockSetup = m.data.plan.setups.find((s) => s.zero === "stock");
  if (stockSetup) {
    const a = toModel(stockSetup, stockSetup.stock.min), b = toModel(stockSetup, stockSetup.stock.max);
    const box = new THREE.Box3(new THREE.Vector3(...a), new THREE.Vector3(...a)).expandByPoint(new THREE.Vector3(...b));
    right.stock = new THREE.Box3Helper(box, 0x3a4350);
    right.scene.add(right.stock);
  }

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(m.positions, 3));
  // RGBA per vertex: what is still to come is faint, except the holes, which would otherwise
  // be lost among thousands of roughing lines.
  m.futureColors = new THREE.BufferAttribute(new Float32Array(m.positions.length / 3 * 4), 4);
  m.doneColors = new THREE.BufferAttribute(new Float32Array(m.positions.length / 3 * 4), 4);
  geometry.setAttribute("color", m.futureColors);
  right.future = new THREE.LineSegments(geometry, new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, depthWrite: false }));
  const done = new THREE.BufferGeometry();
  done.setAttribute("position", geometry.getAttribute("position"));
  done.setAttribute("color", m.doneColors);
  right.done = new THREE.LineSegments(done, new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, depthWrite: false }));
  right.scene.add(right.future, right.done);

  right.focus = new LineSegments2(new LineSegmentsGeometry(), new LineMaterial({ linewidth: 3, color: 0xffffff }));
  right.focus.visible = false;
  right.focus.frustumCulled = false;
  right.scene.add(right.focus);

  const shaft = new THREE.CylinderGeometry(1, 1, 1, 24);
  shaft.rotateX(Math.PI / 2);
  shaft.translate(0, 0, 0.5);
  right.tool = new THREE.Mesh(shaft, new THREE.MeshLambertMaterial({ color: 0xffffff, transparent: true, opacity: 0.55 }));
  right.scene.add(right.tool);
}

function paintToolpaths() {
  if (!model?.futureColors) return;
  const key = activeFocus(), c = new THREE.Color(), rapid = new THREE.Color(RAPID);
  const future = model.futureColors.array, done = model.doneColors.array;
  const focused = [];
  let focusColor = null;
  model.segments.forEach((s, i) => {
    const op = model.operations.get(s.op);
    const matches = key && (s.feature === key || s.op === key);
    if (s.rapid) c.copy(rapid);
    else c.set(op?.color || UNPLANNED);
    const wholePart = op && !op.features?.length;
    let a = s.rapid ? 0.28 : wholePart ? 0.13 : 0.9, b = s.rapid ? 0.45 : wholePart ? 0.3 : 0.9;
    if (key && !matches) { a *= 0.25; b *= 0.2; }
    for (let v = 0; v < 2; v++) {
      future.set([c.r, c.g, c.b, a], 8 * i + 4 * v);
      done.set([c.r, c.g, c.b, b], 8 * i + 4 * v);
    }
    if (matches && !s.rapid) {
      focusColor ??= op?.color || UNPLANNED;
      for (let k = 0; k < 6; k++) focused.push(model.positions[6 * i + k]);
    }
  });
  model.futureColors.needsUpdate = true;
  model.doneColors.needsUpdate = true;
  right.focus.visible = focused.length > 0;
  if (focused.length) {
    right.focus.geometry.dispose();
    right.focus.geometry = new LineSegmentsGeometry().setPositions(focused);
    right.focus.material.color.set(focusColor);
    const operation = model.operations.get(key);  // whole-part paths are too dense to draw bold
    right.focus.material.linewidth = operation && !operation.features?.length ? 1 : 3;
  }
  drawRight();
}

// ------------------------------------------------------------------------------------------
// The feature list, the G-code list and the timeline
// ------------------------------------------------------------------------------------------

const describeTime = (s) => `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;

function rowEvents(el, key) {
  el.dataset.key = key;
  el.addEventListener("pointerenter", () => setFocus(key));
  el.addEventListener("pointerleave", () => setFocus(null));
  el.addEventListener("focus", () => setFocus(key));
  el.addEventListener("blur", () => setFocus(null));
  el.addEventListener("click", () => setPin(pinKey === key ? null : key));
}

function button(className, html, key) {
  const el = document.createElement("button");
  el.type = "button"; el.className = className; el.innerHTML = html;
  rowEvents(el, key);
  return el;
}

const escapeHtml = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function buildFeatureList(m) {
  const host = $("features");
  host.replaceChildren();
  const head = (text) => { const d = document.createElement("div"); d.className = "side-head"; d.textContent = text; host.append(d); };
  const ops = [...m.operations.values()];
  const holes = ops.filter((o) => o.features?.length);
  if (holes.length) head("Features");
  for (const op of holes) {
    const slot = op.kind === "slot", edges = op.kind === "edge_chamfer";
    const count = slot || edges ? 1 : op.features.length;
    const what = slot ? "to size" : edges ? "top edges, not the round" : `${count} ${count === 1 ? "hole" : "holes"}`;
    host.append(button("feature-row", `<span class="swatch" style="background:${op.color}"></span><span><span class="name">${escapeHtml(op.name)}</span><span class="detail">${what}, ${op.time ? describeTime(op.time) : "not posted"}</span></span>`, op.id));
    if (count > 1) {
      const chips = document.createElement("div");
      chips.className = "chips";
      for (const fid of op.features) chips.append(button("chip", escapeHtml(fid.split(":").pop()), fid));
      host.append(chips);
    } else {
      host.lastChild.dataset.keyAlso = op.features[0];
    }
  }
  const whole = ops.filter((o) => !o.features?.length);
  if (whole.length) head("Whole part");
  for (const op of whole) {
    host.append(button("feature-row", `<span class="swatch" style="background:${op.color}"></span><span><span class="name">${escapeHtml(op.name)}</span><span class="detail">${describeTime(op.time)}</span></span>`, op.id));
  }
  const others = [...m.features.values()].filter((f) => !f.operations.length);
  if (others.length) head("Not cut by a feature operation");
  for (const f of others) {
    const note = f.covered ? "covered by roughing" : f.skipped || "not planned";
    host.append(button("feature-row", `<span class="swatch" style="background:${f.color}"></span><span><span class="name">${escapeHtml(f.id)}</span><span class="detail">${escapeHtml(note)}</span></span>`, f.id));
  }
}

function buildTimeline(m) {
  const host = $("segments");
  host.replaceChildren();
  let start = 0;
  for (const op of m.operations.values()) {
    if (!op.time) continue;
    const el = document.createElement("button");
    el.type = "button"; el.className = "segment"; el.style.flex = `${op.span} 1 0`;
    el.setAttribute("aria-label", `Go to ${op.name}`);
    el.title = `${op.name}: ${describeTime(op.time)} est.`;
    el.innerHTML = `<span class="fill" style="background:${op.color};width:0"></span><span class="label">${op.span / m.total > 0.1 ? escapeHtml(op.name) : ""}</span>`;
    const at = start;
    el.dataset.key = op.id; el.dataset.start = start; el.dataset.span = op.span;
    el.addEventListener("pointerenter", () => setFocus(op.id));
    el.addEventListener("pointerleave", () => setFocus(null));
    el.addEventListener("click", () => seek(at));
    host.append(el);
    start += op.span;
  }
}

// What the page says and a saved file would otherwise lose: it goes in the file as comments.
function savedText(m, program) {
  const setup = m.data.plan.setups.find((s) => s.id === program.setup);
  const leftOut = m.data.plan.warnings.filter((w) => w.includes(" left out: "));
  const notes = [
    `step2cam demo, from ${m.data.name}, setup ${program.setup}`,
    "PLACEHOLDER TOOLS AND CUTTING DATA: NOT FOR A REAL MACHINE",
    `X0 Y0 on the minimum corner and Z0 on the top of the ${setup.zero}`,
    ...leftOut.map((w) => `NOT CUT: ${w}`),
  ];
  const comments = notes.map((n) => `; ${n.replace(/[^\x20-\x7e]/g, "?")}`);
  return `${comments.join("\n")}\n${program.nc}`;
}

function buildSaveButtons(m) {
  const host = $("save");
  host.replaceChildren();
  const stem = m.data.name.replace(/\.[^.]*$/, "");
  for (const program of m.data.programs) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = m.data.programs.length > 1 ? `Save ${program.setup}` : "Save G-code";
    button.title = `Save ${stem}-${program.setup}.nc`;
    button.addEventListener("click", () => {
      const link = document.createElement("a");
      link.href = URL.createObjectURL(new Blob([savedText(m, program)], { type: "text/plain" }));
      link.download = `${stem}-${program.setup}.nc`;
      link.click();
      setTimeout(() => URL.revokeObjectURL(link.href), 1000);
    });
    host.append(button);
  }
}

function drawCode() {
  if (!model) return;
  const host = $("code"), rows = $("code-rows");
  const first = Math.max(0, Math.floor(host.scrollTop / ROW_HEIGHT) - 4);
  const count = Math.ceil(host.clientHeight / ROW_HEIGHT) + 8;
  rows.style.top = `${first * ROW_HEIGHT}px`;
  const key = activeFocus(), now = currentLine();
  const out = [];
  for (let i = first; i < Math.min(model.lines.length, first + count); i++) {
    const line = model.lines[i];
    const lit = key && (line.feature === key || line.op === key);
    const cls = i === now ? "now" : lit ? "focus" : key ? "other" : "";
    out.push(`<div class="code-row ${cls}" data-line="${i}"><span class="n">${i + 1}</span><span>${escapeHtml(line.text)}</span></div>`);
  }
  rows.innerHTML = out.join("");
}

function scrollCodeTo(line) {
  const host = $("code");
  const y = line * ROW_HEIGHT;
  if (y < host.scrollTop + ROW_HEIGHT || y > host.scrollTop + host.clientHeight - 2 * ROW_HEIGHT) {
    host.scrollTop = Math.max(0, y - host.clientHeight / 3);
  }
}

// ------------------------------------------------------------------------------------------
// The links: focus and playhead
// ------------------------------------------------------------------------------------------

const activeFocus = () => focusKey || pinKey;

function currentSegment() {
  const s = model.segments;
  if (!s.length) return -1;
  let low = 0, high = s.length - 1;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (s[mid].end < time) low = mid + 1; else high = mid;
  }
  return low;
}

function currentLine() {
  const i = currentSegment();
  return i < 0 ? -1 : model.segments[i].line;
}

function setFocus(key) {
  if (focusKey === key) return;
  focusKey = key;
  applyFocus(true);
}

function setPin(key) {
  pinKey = key;
  focusKey = null;
  applyFocus(true);
}

function linesOf(key) {
  return model.lines.filter((l) => l.feature === key || l.op === key);
}

function applyFocus(scroll) {
  if (!model) return;
  const key = activeFocus();
  for (const el of document.querySelectorAll("[data-key]")) {
    const on = key && (el.dataset.key === key || el.dataset.keyAlso === key);
    el.classList.toggle("is-lit", Boolean(on));
    el.classList.toggle("is-pinned", pinKey !== null && (el.dataset.key === pinKey || el.dataset.keyAlso === pinKey));
  }
  paintToolpaths();
  highlightCad();
  if (scroll) {
    // To what is in focus, or back to the playhead.
    const lines = key ? linesOf(key) : [];
    const target = lines.length ? lines[0].n : currentLine();
    if (target >= 0) scrollCodeTo(target);
  }
  drawCode();
  drawStatus();
}

// The left view lights what is in focus, or else the feature being cut.
let litCad = undefined;
function highlightCad() {
  if (!viewer?.ready || hoverInCad) return;  // the viewer is already lighting what is under the pointer
  let key = activeFocus();
  if (!key) {
    const i = currentSegment();
    key = i >= 0 && (playing || time > 0) ? model.segments[i].feature : null;
  }
  const feature = model.features.get(key);
  const operation = model.operations.get(key);
  const path = feature ? leafByFeature.get(key)
    : operation?.features?.length === 1 ? leafByFeature.get(operation.features[0])
    : operation && !operation.features ? `/${ROOT}/Body`
    : null;
  if (path === litCad) return;
  litCad = path;
  viewer.handleTreeHover(path ?? null);
}

function drawStatus() {
  if (!model) return;
  const key = activeFocus();
  let text;
  if (key) {
    const lines = linesOf(key);
    const name = model.features.has(key) ? describeFeature(model.features.get(key)) : model.operations.get(key)?.name || key;
    text = lines.length ? `${name} · G-code lines ${lines[0].n + 1}–${lines[lines.length - 1].n + 1}` : `${name} · no G-code`;
  } else if (model.segments.length) {
    const s = model.segments[currentSegment()];
    const what = s.feature ? describeFeature(model.features.get(s.feature)) : model.operations.get(s.op)?.name || "";
    text = `${playing || time > 0 ? "Cutting" : "Ready"}: ${what} · line ${s.line + 1}/${model.lines.length} · ${describeTime(s.machining)}/${describeTime(model.machining)} est.`;
  } else {
    text = "No G-code for this part";
  }
  $("status").textContent = text;
}

function describeFeature(f) {
  const r = f.record || {};
  return `${f.id}${r.diameter ? ` ⌀${r.diameter}` : ""}${r.depth ? ` × ${r.depth}` : ""}`;
}

function seek(t) {
  if (!model) return;
  time = Math.max(0, Math.min(model.total, t));
  drawPlayhead();
}

function drawPlayhead() {
  $("scrub").value = model.total ? Math.round(time / model.total * 1000) : 0;
  for (const el of document.querySelectorAll(".segment")) {
    const f = Math.max(0, Math.min(1, (time - el.dataset.start) / el.dataset.span));
    el.firstChild.style.width = `${f * 100}%`;
  }
  const i = currentSegment();
  if (i < 0) { drawStatus(); return; }
  const s = model.segments[i];
  const before = i ? model.segments[i - 1].end : 0;
  const f = s.end > before ? Math.max(0, Math.min(1, (time - before) / (s.end - before))) : 1;
  right.done.geometry.setDrawRange(0, 2 * i + 2);
  const p = model.positions;
  const at = 6 * i;
  const tip = [0, 1, 2].map((k) => p[at + k] + (p[at + 3 + k] - p[at + k]) * f);
  right.tool.position.set(tip[0], tip[1], tip[2]);
  const up = new THREE.Vector3(...s.setup.rotation[2]);
  right.tool.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), up);
  const radius = (s.tool?.diameter || 3) / 2;
  right.tool.scale.set(radius, radius, s.tool?.stickout || 30);
  right.tool.visible = time > 0 && time < model.total;
  drawRight();
  scrollCodeTo(s.line);
  drawCode();
  highlightCad();
  drawStatus();
}

let lastFrame = 0;
function tick(now) {
  if (!playing) return;
  const dt = (now - lastFrame) / 1000;
  lastFrame = now;
  time = Math.min(model.total, time + dt * model.total / PLAYBACK_SECONDS);
  drawPlayhead();
  if (time >= model.total) { setPlaying(false); return; }
  requestAnimationFrame(tick);
}

function setPlaying(on) {
  playing = on && Boolean(model?.segments.length);
  $("play").textContent = playing ? "Pause" : "Play";
  if (playing) {
    if (time >= model.total) time = 0;
    lastFrame = performance.now();
    requestAnimationFrame(tick);
  }
}

// ------------------------------------------------------------------------------------------
// Loading a part
// ------------------------------------------------------------------------------------------

function showBanner(text) {
  $("banner").hidden = !text;
  $("banner").textContent = text || "";
}

function show(data) {
  setPlaying(false);
  focusKey = null; pinKey = null; time = 0; litCad = undefined;
  model = buildModel(data);
  const holes = [...model.features.values()].filter((f) => f.family === "holes").length;
  $("meta").textContent = `${data.name} · ${data.analysis.binding.face_count} faces · ${holes} ${holes === 1 ? "hole" : "holes"} · ${data.plan.setups.length} ${data.plan.setups.length === 1 ? "setup" : "setups"} · ${model.lines.length} lines of G-code`;
  const notes = data.notice ? [data.notice] : [];
  if (data.post_error) notes.push(`Posting failed: ${data.post_error}`);
  notes.push(...data.plan.warnings);
  notes.push(...data.log.filter((l) => /^note: .*no (drill|end mill)/.test(l)).map((l) => l.slice(6)));
  showBanner(notes.join("\n"));
  showPart(model);
  showToolpaths(model);
  buildFeatureList(model);
  buildTimeline(model);
  buildSaveButtons(model);
  $("code-spacer").style.height = `${model.lines.length * ROW_HEIGHT}px`;
  $("code").scrollTop = 0;
  paintToolpaths();
  resizeRight();
  drawPlayhead();
  applyFocus(false);
}

// specify-core, step2cam and camlib all run in this worker, in the visitor's browser.
const worker = new Worker("worker.js", { type: "module" });
let job = 0;

function run(name, bytes) {
  const id = ++job;
  $("busy").hidden = false;
  showBanner("");
  worker.postMessage({ id, name, bytes }, [bytes]);
}

worker.onmessage = ({ data: { id, data, error } }) => {
  if (id !== job) return;  // a newer file was opened meanwhile
  $("busy").hidden = true;
  if (error) {
    console.error(error);
    showBanner(`Could not process that file: ${error}`);
    return;
  }
  show(data);
};
worker.onerror = (event) => {
  $("busy").hidden = true;
  showBanner(`The worker stopped: ${event.message}`);
};

async function openFile(file) {
  run(file.name, await file.arrayBuffer());
}

$("file").addEventListener("change", (event) => {
  if (event.target.files[0]) openFile(event.target.files[0]);
  event.target.value = "";
});
const app = document.querySelector(".app");
window.addEventListener("dragover", (event) => { event.preventDefault(); app.classList.add("dragging"); });
window.addEventListener("dragleave", (event) => { if (!event.relatedTarget) app.classList.remove("dragging"); });
window.addEventListener("drop", (event) => {
  event.preventDefault();
  app.classList.remove("dragging");
  if (event.dataTransfer.files[0]) openFile(event.dataTransfer.files[0]);
});
$("play").addEventListener("click", () => setPlaying(!playing));
$("scrub").addEventListener("input", (event) => { setPlaying(false); seek(event.target.value / 1000 * model.total); });
$("code").addEventListener("scroll", () => requestAnimationFrame(drawCode));
$("code").addEventListener("click", (event) => {
  const row = event.target.closest("[data-line]");
  if (!row) return;
  const line = model.lines[Number(row.dataset.line)];
  if (line.last >= line.first) { setPlaying(false); seek(model.segments[line.last].end); }
});
$("code").addEventListener("pointermove", (event) => {
  const row = event.target.closest("[data-line]");
  const line = row && model.lines[Number(row.dataset.line)];
  setFocusQuiet(line ? line.feature || line.op : null);
});
$("code").addEventListener("pointerleave", () => setFocusQuiet(null));
for (const b of document.querySelectorAll("[data-view]")) {
  b.addEventListener("click", () => viewer?.presetCamera(b.dataset.view, fitZoom(model)));
}

// Hovering the code must not scroll the code away from under the pointer.
function setFocusQuiet(key) {
  if (focusKey === key) return;
  focusKey = key;
  applyFocus(false);
}

new ResizeObserver(() => {
  if (!viewer?.ready) return;
  const box = $("cad").parentElement.getBoundingClientRect();
  viewer.resizeCadView(Math.floor(box.width), 0, Math.floor(box.height), true);
  fitCadMarker(box.width);
  resizeRight();
}).observe($("gcode-view"));

initRight();
initCadLinks();
fetch("plate.step").then((r) => r.arrayBuffer()).then((bytes) => run("plate.step", bytes));
