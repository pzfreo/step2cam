// The step2cam demo page. Draws what worker.js returns: features on the left in
// three-cad-viewer, camlib's toolpaths on the right in three.js, both in model coordinates and
// one camera, with features, operations, toolpaths and camlib's moves linked to each other.

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { LineMaterial } from "three/addons/lines/LineMaterial.js";
import { LineSegments2 } from "three/addons/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/addons/lines/LineSegmentsGeometry.js";
import { Display, Viewer } from "three-cad-viewer";
import { createStock } from "./stock.js";

const $ = (id) => document.getElementById(id);
const FACING = "#a6b0be", ROUGHING = "#e7ebf0", RAPID = "#59626f", BODY = "#8d97a6", UNPLANNED = "#6b7482";
const PALETTE = ["#2dd4bf", "#5aa9ff", "#ffb020", "#f472b6", "#a78bfa", "#a3e635", "#fb7185", "#38bdf8"];
const ROUGHED = "#7fb7a8", NOT_CUT = "#ff7a4d";
// specify-core's descriptions of the part as a whole, not features a person would point at.
const NOT_FEATURES = new Set(["hole_patterns", "step_levels", "risers", "section_recesses", "plates", "thin_wall_bodies", "sheet_metal_bodies", "turned_bodies", "circular_face_patterns"]);
const PLAYBACK_SECONDS = 45;
const ROW_HEIGHT = 20;

let model = null;        // the current part, see buildModel
let focusKey = null;     // what is hovered: a feature id or an operation id
let pinKey = null;       // what was clicked
// Counts an event with GoatCounter (index.html): only that it happened, never anything about the
// file. Does nothing if the counter is blocked or hasn't loaded.
function count(event) {
  try { window.goatcounter?.count?.({ path: event, title: event, event: true }); } catch { /* not counted */ }
}

let time = 0;            // playhead, in estimated machining seconds
let playing = false;
let showRapids = false;  // moves through the air in the toolpath view
let showCurrent = true;  // while playing, the operation being cut stands out
let showStock = true;    // the block, cut away as the program plays
let paintedOp = null;    // the operation the toolpath view was last painted for
let hoveredLine = null;  // the move under the pointer

// ------------------------------------------------------------------------------------------
// The data: operations, features and camlib's moves
// ------------------------------------------------------------------------------------------

function buildModel(data) {
  const operations = new Map(), features = new Map();
  for (const f of data.analysis.features) {
    if (!NOT_FEATURES.has(f.family)) features.set(f.id, { ...f, operations: [], color: UNPLANNED, name: featureName(f) });
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
  describeRoughed(features, data);

  const path = readMoves(data, operations);
  const m = { data, operations, features, ...path };
  for (const line of m.lines) {
    if (line.op && line.seconds) operations.get(line.op).time += line.seconds;
  }
  compressTimeline(m);
  buildRows(m);
  return m;
}

const trim = (x) => String(Math.round(Number(x) * 10) / 10);

function featureName(f) {
  const r = f.record || {};
  switch (f.family) {
    case "holes": return `⌀${trim(r.diameter)} hole`;
    case "slots": return `Slot, ${trim(r.width)} wide`;
    case "pockets": return r.open_sign ? `Notch, ${trim(r.width)} wide × ${trim(r.depth)} deep` : `Pocket, ${trim(r.width)} wide × ${trim(r.depth)} deep`;
    case "v_channels": return `V-channel, ${trim(r.width)} wide × ${trim(r.depth)} deep`;
    case "polygonal_bosses": return r.side_count === 6 ? `Hex boss, ${trim(r.across_flats)} across flats` : `${r.side_count}-sided boss`;
    case "fillets": return r.side === "convex" ? `r${trim(r.radius)} edge round` : `r${trim(r.radius)} fillet`;
    default: return f.family.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
  }
}

// The plan's `covered_by_roughing` knows the feature's faces, not the tool: a Ø12 mill does not
// get into an 8 mm notch. What is too narrow says so, and stands out on the part.
function describeRoughed(features, data) {
  const mills = data.tools.filter((t) => t.type === "end_mill").map((t) => t.diameter);
  const mill = mills.length ? Math.max(...mills) : null;
  const roughs = data.plan.setups.some((s) => s.operations.some((o) => o.kind === "roughing"));
  const wallOnFlat = data.plan.setups.some((s) => s.operations.some((o) => o.kind === "wall_finish" && o.on_flat));
  for (const f of features.values()) {
    if (f.operations.length) continue;
    const width = f.record?.width;
    if (roughs && f.covered && mill && width && width < mill) {
      f.notCut = true;
      f.color = NOT_CUT;
      f.note = `narrower than the ⌀${trim(mill)} mill, so not cut to size`;
    } else if (f.covered) {
      f.color = ROUGHED;
      f.note = f.family === "polygonal_bosses" && wallOnFlat ? "roughed, then its walls finished"
        : f.family === "fillets" ? "roughed, finished only where a wall or flat pass reaches it"
        : "roughed, keeps the roughing allowance";
    } else {
      f.note = f.skipped || "not planned";
    }
  }
}

// What the list of moves shows: camlib's moves, with a heading where each setup and operation starts.
function buildRows(m) {
  const rows = [];
  let program = null, op = null;
  for (const line of m.lines) {
    if (line.program !== program && m.data.programs.length > 1) {
      rows.push({ head: `The ${line.program} setup`, part: true, line: line.n });
    }
    if (line.program !== program || line.op !== op) {
      program = line.program; op = line.op;
      const o = m.operations.get(op), t = line.tool;
      rows.push({ head: `${o?.name ?? op}${t ? ` · T${t.number} ${t.id}, ⌀${trim(t.diameter)}` : ""}`, line: line.n, key: op });
    }
    line.row = rows.length;
    rows.push({ line: line.n });
  }
  m.rows = rows;
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
  if (op.kind === "spot_drill") return "Spot drill" + where;
  if (op.kind === "facing") return "Facing" + where;
  if (op.kind === "roughing") return "Roughing" + where;
  if (op.kind === "counterbore") return `⌀${op.diameter} counterbore` + where;
  if (op.kind === "thread") return `${op.spec} thread` + where;
  if (op.kind === "slot") return `${op.width} × ${op.length} slot` + where;
  if (op.kind === "notch") return `${trim(op.width)} × ${trim(op.length)} ${op.open ? "notch" : "slot"}` + where;
  if (op.kind === "v_channel") return `V-channel, ${trim(op.width)} wide` + where;
  if (op.kind === "flats") return "Finish flats" + where;
  if (op.kind === "wall_finish") return "Finish outside wall" + where;
  if (op.kind === "edge_chamfer") return (op.radius == null ? `${op.width} mm 45° edge chamfer` : `${op.width} chamfer on r${op.radius} edges`) + where;
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

const MOVE_WORDS = {
  rapid: "Rapid", cut: "Cut", plunge: "Plunge", lead: "Lead", link: "Link", retract: "Retract", tap: "Tap", shift: "Shift", thread: "Thread",
};
const xyz = (p) => `X${p[0].toFixed(3)} Y${p[1].toFixed(3)} Z${p[2].toFixed(3)}`;

// camlib's moves, each drawn as the straight pieces its points make (an arc's points lie on it).
function readMoves(data, operations) {
  const toolById = new Map(data.tools.map((t) => [t.id, t]));
  const rapidRate = data.rapid_mm_min || 5000;
  const lines = [], segments = [], positions = [], local = [];
  let machining = 0;

  for (const program of data.programs) {
    const setup = data.plan.setups.find((s) => s.id === program.setup);
    for (let move of program.moves) {
      const end = move.points[move.points.length - 1];
      if (move.op?.includes("-spot_drill-")) move = { ...move, op: move.op.split("-spot_drill-")[0] + "-spot_drill" };
      const tool = toolById.get(move.tool) || null;
      const curve = move.curve === "line" ? "" : ` ${move.curve}`;
      const text = `${MOVE_WORDS[move.kind] || move.kind}${curve}  ${xyz(end)}${move.feed ? ` F${move.feed}` : ""}`;
      const line = { text, move, tool, program: program.setup, n: lines.length, first: segments.length, last: segments.length - 1 };
      lines.push(line);
      line.op = move.op;
      line.feature = nearestFeature(operations.get(move.op), end);
      line.rapid = move.kind === "rapid";
      const rate = line.rapid ? rapidRate : move.feed || rapidRate;
      let length = 0;
      for (let i = 1; i < move.points.length; i++) {
        const from = move.points[i - 1], to = move.points[i];
        const d = Math.hypot(to[0] - from[0], to[1] - from[1], to[2] - from[2]);
        length += d;
        const seconds = d / rate * 60;
        machining += seconds;
        segments.push({ line: line.n, op: line.op, feature: line.feature, rapid: line.rapid, tool, setup, seconds, machining });
        positions.push(...toModel(setup, from), ...toModel(setup, to));
        local.push(...from, ...to);
      }
      line.seconds = length / rate * 60;
      line.last = segments.length - 1;
    }
  }
  return { lines, segments, positions: new Float32Array(positions), local: new Float32Array(local), machining };
}

function nearestFeature(op, point) {
  if (op?.kind === "slot" || op?.kind === "notch" || op?.kind === "v_channel") return op.features[0];
  if (op?.kind === "edge_chamfer") return null;
  if (!op?.centers) return null;
  let best = null, bestD = Infinity;
  op.centers.forEach((c, i) => {
    const d = Math.hypot(point[0] - c[0], point[1] - c[1]);
    if (d < bestD) { bestD = d; best = op.features[i]; }
  });
  return bestD <= (op.diameter ?? op.start_diameter) ? best : null;
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
  if (!right.renderer) return;
  right.block?.aim(right.camera);
  right.renderer.render(right.scene, right.camera);
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
  if (right.block) {
    right.scene.remove(right.block.mesh);
    right.block.dispose();
    right.block = null;
  }
  try {
    right.block = createStock(m, toModel);
    right.block.mesh.visible = showStock;
    right.scene.add(right.block.mesh);
  } catch (error) {
    console.error("no stock view:", error);
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
  // While playing, the operation being cut stands out from what is around it.
  const index = currentSegment();
  const current = showCurrent && !key && (playing || time > 0) && index >= 0 ? model.segments[index].op : null;
  paintedOp = current;
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
    if (current) {
      if (s.op === current) { a = s.rapid ? a : 0.75; b = s.rapid ? b : 0.95; }
      else { a *= 0.2; b *= 0.25; }
    }
    if (s.rapid && !showRapids) { a = 0; b = 0; }
    // With the block drawn, what has been cut is shown by the block; its paths would only hide it.
    if (showStock && right.block && s.op !== current && !matches) b = 0;
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
// The feature list, the list of moves and the timeline
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
    const slot = op.kind === "slot" || op.kind === "notch" || op.kind === "v_channel", edges = op.kind === "edge_chamfer";
    const count = slot || edges ? 1 : op.features.length;
    const tool = [...m.segments].find((s) => s.op === op.id)?.tool;
    const what = op.kind === "notch" ? `to size, corners r${tool ? trim(tool.diameter / 2) : "?"}` : op.kind === "v_channel" ? `${tool ? `${tool.angle}° ${tool.id}` : "V-bit"}, to the root` : slot ? "to size" : edges ? "top edges, not the round" : `${count} ${count === 1 ? "hole" : "holes"}`;
    host.append(button("feature-row", `<span class="swatch" style="background:${op.color}"></span><span><span class="name">${escapeHtml(op.name)}</span><span class="detail">${what}, ${op.time ? describeTime(op.time) : "not run"}</span></span>`, op.id));
    if (count > 1) {
      const chips = document.createElement("div");
      chips.className = "chips";
      op.features.forEach((fid, i) => {
        const chip = button("chip", String(i + 1), fid);
        chip.setAttribute("aria-label", `${op.name}, number ${i + 1}`);
        chips.append(chip);
      });
      host.append(chips);
    } else {
      host.lastChild.dataset.keyAlso = op.features[0];
    }
  }
  // Recognised, with no operation of their own: like ones together, what was not cut first.
  const others = [...m.features.values()].filter((f) => !f.operations.length);
  const rounds = (f) => f.family === "fillets" || f.family === "chamfers" || f.family === "blends";
  const groupOf = (list) => {
    const groups = new Map();
    for (const f of list) {
      const key = `${f.name}|${f.note}`;
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(f);
    }
    return [...groups.values()];
  };
  const row = (members) => {
    const f = members[0], name = members.length > 1 ? `${f.name} × ${members.length}` : f.name;
    const html = `<span class="swatch" style="background:${f.color}"></span><span><span class="name">${escapeHtml(name)}</span><span class="detail">${escapeHtml(f.note)}</span></span>`;
    if (members.length === 1) {
      host.append(button(`feature-row${f.notCut ? " not-cut" : ""}`, html, f.id));
      return;
    }
    const title = document.createElement("div");
    title.className = `feature-row group${f.notCut ? " not-cut" : ""}`;
    title.innerHTML = html;
    host.append(title);
    const chips = document.createElement("div");
    chips.className = "chips";
    members.forEach((member, i) => chips.append(button("chip", String(i + 1), member.id)));
    host.append(chips);
  };
  const section = (text, list) => {
    if (!list.length) return;
    head(text);
    groupOf(list).forEach(row);
  };
  section("Recognised, not cut to size", others.filter((f) => f.notCut));
  section("Recognised, left to roughing", others.filter((f) => !f.notCut && f.covered && !rounds(f)));
  section("Recognised, not cut", others.filter((f) => !f.notCut && !f.covered && !rounds(f)));

  const whole = ops.filter((o) => !o.features?.length);
  if (whole.length) head("Whole part");
  for (const op of whole) {
    host.append(button("feature-row", `<span class="swatch" style="background:${op.color}"></span><span><span class="name">${escapeHtml(op.name)}</span><span class="detail">${describeTime(op.time)}</span></span>`, op.id));
  }
  section("Edges", others.filter(rounds));
}

function shortName(op) {
  const names = { spot_drill: "Spot drill", facing: "Facing", roughing: "Roughing", flats: "Flats", wall_finish: "Walls", edge_chamfer: "Chamfer", slot: "Slot", notch: op.open ? "Notch" : "Slot", v_channel: "V", thread: op.spec, counterbore: "C'bore" };
  return names[op.kind] || `⌀${op.diameter}`;
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
    el.innerHTML = `<span class="fill" style="background:${op.color};width:0"></span><span class="label">${op.span / m.total > 0.04 ? escapeHtml(shortName(op)) : ""}</span>`;
    const at = start;
    el.dataset.key = op.id; el.dataset.start = start; el.dataset.span = op.span;
    el.addEventListener("pointerenter", () => setFocus(op.id));
    el.addEventListener("pointerleave", () => setFocus(null));
    el.addEventListener("click", () => seek(at));
    host.append(el);
    start += op.span;
  }
}

// ------------------------------------------------------------------------------------------
// The process plan: what the planner decided and why, for a person to check
// ------------------------------------------------------------------------------------------

// The rules the planner cites, with their rationale and sources (rulebase/rules.json).
let rulesById = null;
async function loadRules() {
  if (rulesById) return rulesById;
  try {
    const doc = await (await fetch("rules.json")).json();
    rulesById = new Map(doc.rules.map((r) => [`${r.id}@${r.version}`, r]));
  } catch {
    rulesById = new Map();
  }
  return rulesById;
}

const STAGES = { machine: "Machine", requirements: "Requirement", setup: "Setup", process: "Process", sequence: "Order", feasibility: "Feasibility", tools: "Tool", execution: "Execution" };

function describeSource(src) {
  const what = src.kind === "standard" ? src.ref : src.kind === "practice" ? "recognised practice" : "step2cam's own judgement";
  const via = src.via ? ` (${src.via})` : "";
  const note = src.note ? `: ${src.note}` : "";
  return `${what}${note}${via}`;
}

function decisionItem(d, rules) {
  const li = document.createElement("li");
  li.className = `decision${d.superseded_by ? " superseded" : ""}`;
  const rule = rules.get(d.rule);
  const rejected = (d.considered || []).filter((c) => c.rejected);
  li.innerHTML = `<span class="says">${escapeHtml(d.says)}${d.times ? ` <span class="tag">× ${d.times}</span>` : ""}${d.superseded_by ? ` <span class="tag">replaced by ${escapeHtml(d.superseded_by)}</span>` : ""}</span>`
    + `<span class="rule">${escapeHtml(STAGES[d.stage] || d.stage)} · ${escapeHtml(d.rule)} · ${escapeHtml(d.id)}</span>`
    + (rejected.length ? `<ul>${rejected.map((c) => `<li>${escapeHtml(c.candidate)}: ${escapeHtml(c.rejected)}</li>`).join("")}</ul>` : "");
  if (rule) {
    const more = document.createElement("details");
    more.className = "rule-detail";
    more.innerHTML = `<summary>The rule</summary><p>${escapeHtml(rule.rationale)}</p>`
      + `<p>Source: ${rule.sources.map((s) => escapeHtml(describeSource(s))).join("; ")}</p>`
      + (rule.data ? `<p>Kept as data in ${rule.data.map(escapeHtml).join(", ")}</p>` : "")
      + `<p>Applied in ${escapeHtml(rule.code)}</p>`;
    li.append(more);
  }
  return li;
}

function whyList(ids, decisions, rules, label = "Why") {
  const found = ids.map((id) => decisions.get(id)).filter(Boolean);
  const box = document.createElement("details");
  box.className = "why";
  box.innerHTML = `<summary>${escapeHtml(label)} (${found.length} ${found.length === 1 ? "decision" : "decisions"})</summary>`;
  const list = document.createElement("ol");
  for (const d of found) list.append(decisionItem(d, rules));
  box.append(list);
  return box;
}

function facts(rows) {
  const dl = document.createElement("dl");
  dl.className = "plan-facts";
  for (const [k, v] of rows) {
    if (v == null || v === "") continue;
    dl.insertAdjacentHTML("beforeend", `<dt>${escapeHtml(k)}</dt><dd>${v}</dd>`);
  }
  return dl;
}

function describeTechnology(t) {
  if (!t) return null;
  const parts = [];
  if (t.spindle_rpm != null) parts.push(`${t.spindle_rpm} rpm`);
  if (t.feed_mm_min != null) parts.push(`feed ${t.feed_mm_min} mm/min`);
  if (t.plunge_mm_min != null) parts.push(`plunge ${t.plunge_mm_min}`);
  if (t.stepdown_mm != null) parts.push(`stepdown ${t.stepdown_mm} mm`);
  if (t.stepover_mm != null) parts.push(`stepover ${t.stepover_mm} mm`);
  const from = t.source === "tool" ? "the tool's own" : `the tool's data for group ${t.source.split(".")[1]}`;
  return `${escapeHtml(parts.join(", "))} <span class="tag">${escapeHtml(from)}</span>`;
}

async function buildPlan(m) {
  const host = $("plan-body");
  host.replaceChildren();
  const plan = m.data.plan, rules = await loadRules();
  if (model !== m) return;  // another part was opened meanwhile
  const decisions = new Map((plan.decisions || []).map((d) => [d.id, d]));
  const head = (text) => host.insertAdjacentHTML("beforeend", `<h3>${escapeHtml(text)}</h3>`);
  const fact = (html) => host.insertAdjacentHTML("beforeend", `<p class="plan-fact">${html}</p>`);
  showPlanList();
  host.insertAdjacentHTML("beforeend", `<p class="plan-fact plan-lead"><b>A draft to check, not a plan to run.</b> What the planner decided, in the order it would be cut, and why. Every choice names the written rule it was made by. Open an operation to see its decisions; <b>Full view</b> shows the tools it considered and the rules themselves.</p>`);

  head("Part");
  const material = plan.material || {};
  const from = material.override ? "the tool library's override" : material.specified ? "Specify" : null;
  fact(`<b>Material:</b> ${material.used ? `${escapeHtml(material.used)} (from ${from})` : "not given"}${material.iso513 ? ` <span class="tag">ISO 513 group ${escapeHtml(material.iso513)}</span>` : material.used ? ' <span class="tag warn">no ISO 513 group</span>' : ""}`);
  const materialWhy = (plan.decisions || []).filter((d) => d.rule.startsWith("requirement.material.group@")).map((d) => d.id);
  if (materialWhy.length) host.append(whyList(materialWhy, decisions, rules));
  const ran = new Set(m.data.programs.map((p) => p.setup));
  fact(`<b>${plan.setups.length} ${plan.setups.length === 1 ? "setup" : "setups"}</b>, ${[...m.operations.values()].length} operations, ${plan.skipped.length} features not cut, ${plan.warnings.length} warnings.`);

  for (const setup of plan.setups) {
    const lo = setup.stock.min, hi = setup.stock.max;
    head(`${setup.id} setup${ran.has(setup.id) ? "" : " · not run"}`);
    const onMachine = (plan.decisions || []).find((d) => d.rule.startsWith("machine.for_setup@") && d.subject === setup.id);
    fact(`${onMachine ? `${escapeHtml(onMachine.says[0].toUpperCase() + onMachine.says.slice(1))}. ` : ""}Tool comes down −Z; zero on the ${setup.zero === "stock" ? "stock" : "part's box"}, stock ${trim(hi[0] - lo[0])} × ${trim(hi[1] - lo[1])} × ${trim(hi[2] - lo[2])} mm.`);
    setup.operations.forEach((op, n) => {
      const key = `${setup.id}-${n + 1}-${op.kind}`, o = m.operations.get(key);
      const card = document.createElement("section");
      card.className = `plan-op${o && o.time ? "" : " unused"}`;
      const headBtn = button("plan-op-head", `<span class="step">${n + 1}.</span><span class="swatch" style="background:${o?.color || UNPLANNED}"></span><span class="name">${escapeHtml(o?.name || op.kind)} <span class="op-meta">· ${escapeHtml(op.tool || "no tool")}${o && o.time ? ` · ${describeTime(o.time)}` : ""}</span>${o && o.time ? "" : ' <span class="tag warn">not run</span>'}</span>`, key);
      card.append(headBtn);
      for (const d of planWarnings(m).filter((d) => op.because?.includes(d.id))) {
        const note = document.createElement("button");
        note.type = "button"; note.className = "operation-warning";
        note.textContent = warningText(m, d, true);
        linkWarning(note, m, d);
        card.append(note);
      }
      const iso = op.iso14649;
      const open = document.createElement("details");
      open.className = "why";
      open.innerHTML = `<summary>Why</summary>`;
      headBtn.addEventListener("click", () => { open.open = pinKey === key; });
      card.append(open);
      open.append(facts([
        ["Tool", op.tool ? escapeHtml(op.tool) : `<span class="tag warn">none</span> ${escapeHtml(op.tool_note || "")}`],
        ["Cutting data", describeTechnology(op.technology)],
        ["Features", op.features?.length ? escapeHtml(op.features.map((id) => m.features.get(id)?.name || "feature").join(", ")) : null],
        ["ISO 14649", iso ? `${escapeHtml(iso.operation || "no operation")} on ${escapeHtml(iso.feature)}${iso.bottom_condition ? `, ${escapeHtml(iso.bottom_condition)}` : ""}${iso.exact ? "" : ' <span class="tag">closest</span>'}` : null],
        ["Time", o && o.time ? `${describeTime(o.time)} est.` : null],
      ]));
      if (op.because?.length) {
        const list = document.createElement("ol");
        for (const d of folded(op.because.map((id) => decisions.get(id)).filter(Boolean))) list.append(decisionItem(d, rules));
        open.append(list);
      }
      const full = document.createElement("button");
      full.type = "button"; full.className = "full-view"; full.textContent = "Full view: tools considered and the rules";
      full.addEventListener("click", () => showDetail(m, setup, op, n, key));
      open.append(full);
      host.append(card);
    });
  }

  // What was refused by camlib, not cut, or needs a look: each with the rule behind it.
  const refused = (plan.decisions || []).filter((d) => d.rule.startsWith("execution.refused@") && d.outcome === "removed");
  if (refused.length) {
    head("Refused by camlib");
    for (const d of refused) {
      const note = document.createElement("div");
      note.className = "plan-note";
      note.innerHTML = `<span class="who">${escapeHtml(d.subject)}</span>: ${escapeHtml(d.says)}`;
      note.append(whyList([d.id], decisions, rules, "Rule"));
      host.append(note);
    }
  }
  if (plan.skipped.length) {
    head("Not cut");
    for (const s of plan.skipped) {
      const note = document.createElement("div");
      note.className = "plan-note skip";
      const f = m.features.get(s.feature);
      note.innerHTML = `<span class="who">${escapeHtml(f?.name || s.feature)}</span> <span class="tag">${escapeHtml(s.feature)}</span>: ${escapeHtml(s.reason)}`;
      if (s.decision) note.append(whyList([s.decision], decisions, rules, "Rule"));
      if (f) rowEvents(note, s.feature);
      host.append(note);
    }
  }
  if (plan.warnings.length) {
    head("Warnings");
    for (const d of planWarnings(m)) {
      const note = document.createElement("div");
      note.className = "plan-note";
      note.textContent = warningText(m, d);
      note.append(whyList([d.id], decisions, rules, "Rule"));
      linkWarning(note, m, d);
      host.append(note);
    }
  }

  applyFocus(false);
}

function showPlanList() {
  $("plan-detail").hidden = true;
  $("plan-body").hidden = false;
}

// Decisions that say the same by the same rule (one per hole, say), folded into one with a count.
function folded(ds) {
  const out = [];
  for (const d of ds) {
    const same = out.find((x) => x.rule === d.rule && x.says === d.says && !x.superseded_by && !d.superseded_by);
    if (same) same.times = (same.times || 1) + 1; else out.push({ ...d });
  }
  return out;
}

// The fuller view of one operation: each decision behind it, the tools it weighed, and the rules.
async function showDetail(m, setup, op, n, key) {
  const host = $("plan-detail"), rules = await loadRules();
  const plan = m.data.plan, o = m.operations.get(key);
  const decisions = new Map((plan.decisions || []).map((d) => [d.id, d]));
  const uses = new Map();
  for (const d of plan.decisions || []) uses.set(d.rule, (uses.get(d.rule) || 0) + 1);
  host.replaceChildren();
  const back = document.createElement("button");
  back.type = "button"; back.className = "detail-back"; back.textContent = "Back to the plan";
  back.addEventListener("click", () => { showPlanList(); $("plan-body").querySelector(`[data-key="${CSS.escape(key)}"]`)?.focus(); });
  host.append(back);
  host.insertAdjacentHTML("beforeend", `<h2 class="detail-title"><span class="swatch" style="background:${o?.color || UNPLANNED}"></span>${escapeHtml(o?.name || op.kind)}</h2><p class="plan-fact" style="color:var(--muted)">${escapeHtml(setup.id)} setup · step ${n + 1} of ${setup.operations.length}</p>`);
  const iso = op.iso14649;
  host.append(facts([
    ["Tool", op.tool ? escapeHtml(op.tool) : `<span class="tag warn">none</span> ${escapeHtml(op.tool_note || "")}`],
    ["Cutting data", describeTechnology(op.technology)],
    ["Features", op.features?.length ? escapeHtml(op.features.map((id) => m.features.get(id)?.name || "feature").join(", ")) : null],
    ["ISO 14649", iso ? `${escapeHtml(iso.operation || "no operation")} on ${escapeHtml(iso.feature)}${iso.exact ? "" : ' <span class="tag">closest</span>'}${iso.note ? `<br><span style="color:var(--muted)">${escapeHtml(iso.note)}</span>` : ""}` : null],
    ["Time", o && o.time ? `${describeTime(o.time)} est.` : null],
  ]));
  const found = folded((op.because || []).map((id) => decisions.get(id)).filter(Boolean));
  host.insertAdjacentHTML("beforeend", `<h3>Why</h3>`);
  const list = document.createElement("ol");
  list.className = "detail-decisions";
  found.forEach((d, i) => {
    const li = document.createElement("li");
    const considered = d.considered || [];
    const chosen = typeof d.chosen === "string" ? d.chosen : null;
    li.innerHTML = `<span class="n">${i + 1}</span><div><div${d.superseded_by ? ' style="text-decoration:line-through;color:var(--muted)"' : ""}>${escapeHtml(d.says)}${d.times ? ` <span class="tag">× ${d.times}</span>` : ""}</div><div class="rule-id" style="font-family:var(--mono);font-size:11.5px;color:var(--muted)">${escapeHtml(STAGES[d.stage] || d.stage)} · ${escapeHtml(d.rule)}</div>`
      + (considered.length && d.stage === "tools" ? `<div class="considered"><span style="grid-column:1/-1;color:var(--muted);font-size:12px">Tools considered</span>${chosen ? `<span class="who chosen">${escapeHtml(chosen)}</span><span>chosen</span>` : ""}${considered.map((c) => `<span class="who">${escapeHtml(c.candidate)}</span><span>${escapeHtml(c.rejected)}</span>`).join("")}</div>` : "")
      + (considered.length && d.stage === "setup" ? `<div class="considered"><span style="grid-column:1/-1;color:var(--muted);font-size:12px">Setups turned down</span>${considered.map((c) => `<span class="who">${escapeHtml(c.candidate)}</span><span>${escapeHtml(c.rejected)}</span>`).join("")}</div>` : "")
      + `</div>`;
    list.append(li);
  });
  host.append(list);
  const cited = [...new Set(found.map((d) => d.rule))];
  host.insertAdjacentHTML("beforeend", `<h3>The rules</h3>`);
  for (const id of cited) {
    const rule = rules.get(id);
    const card = document.createElement("section");
    card.className = "rule-card";
    card.innerHTML = `<div><b>${escapeHtml(rule ? STAGES[rule.stage] || rule.stage : "Rule")}</b> <span class="rule-id">${escapeHtml(id)}</span></div>`
      + (rule ? `<p>${escapeHtml(rule.rationale)}</p><dl><dt>Source</dt><dd>${rule.sources.map((x) => escapeHtml(describeSource(x))).join("; ")}</dd>${rule.data ? `<dt>Kept as data</dt><dd>${rule.data.map(escapeHtml).join(", ")}</dd>` : ""}<dt>Applied in</dt><dd>${escapeHtml(rule.code)}</dd><dt>In this plan</dt><dd>${uses.get(id)} ${uses.get(id) === 1 ? "decision" : "decisions"}</dd></dl>` : "");
    host.append(card);
  }
  $("plan-body").hidden = true;
  host.hidden = false;
  host.scrollTop = 0;
  back.focus();
}

// The three steps above the panes, in numbers for this part.
async function showStory(m) {
  const rules = await loadRules();
  if (model !== m) return;
  const data = m.data, plan = data.plan;
  const count = (n, one, many = `${one}s`) => `${n.toLocaleString("en-GB")} ${n === 1 ? one : many}`;
  const families = new Map();
  for (const f of m.features.values()) families.set(f.family, (families.get(f.family) || 0) + 1);
  const named = [["holes", "hole"], ["slots", "slot"], ["pockets", "notch", "notches"], ["v_channels", "V-channel"], ["fillets", "edge round"]]
    .filter(([fam]) => families.get(fam)).map(([fam, one, many]) => count(families.get(fam), one, many));
  $("story-features").textContent = `Reads the solid's ${count(data.analysis.binding.face_count, "face")} and finds ${count(m.features.size, "feature")}${named.length ? `: ${named.join(", ")}` : ""}.`;
  const ops = plan.setups.flatMap((s) => s.operations);
  const tools = new Set(ops.map((o) => o.tool).filter(Boolean));
  const decisions = plan.decisions || [];
  $("story-plan").textContent = `Decides how to make it: ${count(plan.setups.length, "setup")}, ${count(ops.length, "operation")}, ${count(tools.size, "tool")}. ${count(decisions.length, "decision")}, each traced to one of ${rules.size || "its"} written rules.`;
  const checked = data.programs.filter((p) => p.material);
  const check = data.error ? "camlib could not run it." : checked.length ? "then cuts a virtual block and compares it with the part: no gouge found." : "but the finished stock could not be compared with the part.";
  $("story-toolpaths").textContent = data.error ? "camlib could not run the plan: see the message above." : `camlib turns the plan into ${count(m.lines.length, "toolpath move")}, ${check}`;
  $("story").hidden = false;
}

function savePlan() {
  if (!model) return;
  count("save-plan");
  const name = (model.data.name || "part").replace(/\.(step|stp)$/i, "");
  const blob = new Blob([JSON.stringify(model.data.plan, null, 1)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `${name}-plan.json`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

// The camlib job for each setup: what camlib's own posts write G-code from.
function buildSaveButtons(m) {
  const host = $("save");
  host.replaceChildren();
  const stem = m.data.name.replace(/\.[^.]*$/, "");
  for (const program of m.data.programs) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = m.data.programs.length > 1 ? `Save ${program.setup} job` : "Save camlib job";
    button.title = `Save ${stem}-${program.setup}.camlib-job.json: the setup as a camlib job, for camlib's posts to write G-code from`;
    button.addEventListener("click", () => {
      count("save-job");
      const link = document.createElement("a");
      link.href = URL.createObjectURL(new Blob([JSON.stringify(program.job)], { type: "application/json" }));
      link.download = `${stem}-${program.setup}.camlib-job.json`;
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
  for (let i = first; i < Math.min(model.rows.length, first + count); i++) {
    const row = model.rows[i];
    if (row.head) {
      out.push(`<div class="code-head${row.part ? " part" : ""}" data-line="${row.line}" title="${escapeHtml(row.head)}">${escapeHtml(row.head)}</div>`);
      continue;
    }
    const line = model.lines[row.line];
    const lit = key && (line.feature === key || line.op === key);
    const cls = row.line === now ? "now" : lit ? "focus" : key ? "other" : "";
    out.push(`<div class="code-row ${cls}" data-line="${row.line}"><span class="n">${row.line + 1}</span><span>${coloured(line.text)}</span></div>`);
  }
  rows.innerHTML = out.join("");
}

function coloured(text) {
  return escapeHtml(text).replace(/\b([A-Z])(-?[\d.]+)/g, (word, letter) => {
    const kind = letter === "G" ? "w-g" : letter === "M" ? "w-m" : letter === "F" || letter === "S" ? "w-f" : letter === "T" || letter === "H" ? "w-t" : "w-c";
    return `<span class="${kind}">${word}</span>`;
  });
}

const MOVE_EXPLAINED = {
  rapid: "Rapid move through the air", cut: "Cut", plunge: "Feed down into the material", lead: "Lead into or out of the cut",
  link: "Link between cuts, at feed", retract: "Retract from the cut", tap: "Tap", shift: "Shift with the spindle stopped", thread: "Cut a thread",
};

// One of camlib's moves in words, for the panel under the list.
function explain(line) {
  const { move } = line;
  const shape = move.curve === "line" ? "in a straight line" : `along ${move.curve === "arc" ? "an arc" : "a helix"}`;
  const feed = move.feed ? ` at ${move.feed} mm/min` : "";
  const tool = line.tool ? `, ${line.tool.id}` : "";
  return `${MOVE_EXPLAINED[move.kind] || move.kind} ${shape} to ${xyz(move.points[move.points.length - 1])}${feed}${tool}`;
}

function drawExplain() {
  const host = $("code-explain");
  const n = hoveredLine ?? currentLine();
  const line = model?.lines[n];
  host.innerHTML = line ? `<code>${n + 1}: ${coloured(line.text)}</code><span>${escapeHtml(explain(line))}</span>` : "";
}

function scrollCodeTo(line) {
  const host = $("code");
  const y = (model.lines[line]?.row ?? 0) * ROW_HEIGHT;
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
    const matches = (target) => target && (el.dataset.key === target || el.dataset.keyAlso === target || model.operations.get(target)?.features?.includes(el.dataset.key));
    const on = matches(key);
    el.classList.toggle("is-lit", Boolean(on));
    el.classList.toggle("is-pinned", Boolean(matches(pinKey)));
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
  drawExplain();
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
  const paths = feature ? [leafByFeature.get(key)]
    : operation?.features?.length ? [...new Set(operation.features.map((id) => leafByFeature.get(id)).filter(Boolean))]
    : operation && !operation.features ? [`/${ROOT}/Body`] : [];
  const identity = paths.join("|");
  if (identity === litCad) return;
  litCad = identity;
  viewer.clearSelection();
  viewer.handleTreeHover(paths.length === 1 ? paths[0] : null);
  if (paths.length > 1) {
    const highlight = viewer.rendered.nestedGroup.highlight;
    for (const path of paths) highlight.selectSolid(path, true);
    viewer.update(true, false);
  }
}

function drawStatus() {
  if (!model) return;
  const key = activeFocus();
  let text;
  if (key) {
    const lines = linesOf(key);
    const name = model.features.has(key) ? describeFeature(model.features.get(key)) : model.operations.get(key)?.name || key;
    text = lines.length ? `${name} · moves ${lines[0].n + 1}–${lines[lines.length - 1].n + 1}` : `${name} · no toolpath`;
  } else if (model.segments.length) {
    const s = model.segments[currentSegment()];
    const what = s.feature ? describeFeature(model.features.get(s.feature)) : model.operations.get(s.op)?.name || "";
    text = `${playing || time > 0 ? "Cutting" : "Ready"}: ${what} · move ${s.line + 1}/${model.lines.length} · ${describeTime(s.machining)}/${describeTime(model.machining)} est.`;
  } else {
    text = "No toolpaths for this part";
  }
  $("status").textContent = text;
}

function describeFeature(f) {
  const r = f.record || {};
  if (f.family !== "holes") return f.name;
  return `⌀${trim(r.diameter)} hole${r.depth ? `, ${trim(r.depth)} deep` : ""}`;
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
  if (showStock) right.block?.update(time > 0 ? i : 0);
  // With the block drawn, paths already cut are shown by the block, not by their lines.
  right.future.geometry.setDrawRange(showStock && right.block && time > 0 ? 2 * i : 0, Infinity);
  const p = model.positions;
  const at = 6 * i;
  const tip = [0, 1, 2].map((k) => p[at + k] + (p[at + 3 + k] - p[at + k]) * f);
  right.tool.position.set(tip[0], tip[1], tip[2]);
  const up = new THREE.Vector3(...s.setup.rotation[2]);
  right.tool.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), up);
  const radius = (s.tool?.diameter || 3) / 2;
  right.tool.scale.set(radius, radius, s.tool?.stickout || 30);
  right.tool.visible = time > 0 && time < model.total;
  const current = showCurrent && !activeFocus() && (playing || time > 0) ? s.op : null;
  if (current !== paintedOp) paintToolpaths();
  drawRight();
  scrollCodeTo(s.line);
  drawCode();
  drawExplain();
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
    count("play");
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
  $("banner").classList.remove("phone-banner-open");
  if (text) addPhoneBannerToggle();
}

function addPhoneBannerToggle() {
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "phone-only phone-banner-toggle";
  toggle.textContent = "⚠ " + $("banner").textContent;
  toggle.setAttribute("aria-expanded", "false");
  toggle.addEventListener("click", () => {
    const open = $("banner").classList.toggle("phone-banner-open");
    toggle.setAttribute("aria-expanded", String(open));
  });
  $("banner").prepend(toggle);
}

// The check's result in words: the headline, and what it cannot see behind a toggle.
function showChecks(data) {
  const host = $("checks");
  const programs = data.error ? [] : data.programs;
  host.hidden = !programs.length;
  host.open = false;
  if (!programs.length) return;
  const setupName = (id) => `${id[0].toUpperCase()}${id.slice(1)} setup`;
  const checked = programs.filter((p) => p.material), unchecked = programs.filter((p) => !p.material);
  const headline = [];
  if (checked.length) headline.push(`<span class="ok">✓ ${checked.map((p) => setupName(p.setup)).join(", ")}: compared with the part, no gouge found.</span>`);
  if (unchecked.length) headline.push(`<span class="unchecked">${unchecked.map((p) => setupName(p.setup)).join(", ")}: not compared with the part.</span>`);
  $("checks-summary").innerHTML = headline.join(" ");
  const compact = document.createElement("span");
  compact.className = "phone-only";
  compact.textContent = checked.length ? "✓ no gouge" : "⚠ unchecked";
  $("checks-summary").append(compact);
  const number = (x) => Math.round(x).toLocaleString("en-GB");
  const items = [];
  for (const p of checked) {
    const { limits, gouge, residual } = p.material;
    const where = checked.length > 1 ? `${setupName(p.setup)}: ` : "";
    const say = (text) => items.push(where ? where + text : text[0].toUpperCase() + text.slice(1));
    const cell = limits.sideways_mm;
    say(`too small for the check to see: errors under about ${limits.height_mm.toFixed(2)} mm in height or ${cell.toFixed(2)} mm sideways, and misplaced cuts narrower than about ${(4 * cell).toFixed(1)} mm.`);
    say(`it can't tell for ${number(gouge.volume_mm3[1])} mm³ beside walls and curved faces, because it sees the part from above.`);
    say(`at least ${number(residual.volume_mm3[0])} mm³ of stock is still there. That includes the web left around the part, and anything a later setup cuts.`);
    if (limits.before_threads) say(`threads are not compared, because they go beyond the bore the model shows.`);
  }
  if (unchecked.length) items.push("Not compared: only the setup with roughing holds the part's model in its position, so side holes and the like rely on the other checks.");
  $("checks-detail").innerHTML = `<ul>${items.map((i) => `<li>${escapeHtml(i)}</li>`).join("")}</ul>`;
}

// Warning identity and links come from the trace, never from warning prose.
function planWarnings(m) {
  return (m.data.plan.decisions || []).filter((d) => d.outcome === "warning" && !d.superseded_by);
}

function warningOperations(m, d) {
  return [...m.operations.values()].filter((o) => o.because?.includes(d.id));
}

function warningText(m, d, brief = false) {
  let text = d.says.replace("no source-admitted entry/route for component 0; choose a smaller radius or another entry strategy", "camlib found no way for that tool to enter it");
  if (brief && d.depth_diameter_ratio != null) text = `${Number(d.depth_diameter_ratio).toFixed(1)} × diameter deep: full-retract pecks`;
  else if (brief && d.radius != null) text = `Inside corners left r${trim(d.radius)} (${d.model_radius ? `model r${trim(d.model_radius)}` : "sharp in the model"})`;
  // Older subject-specific warnings still include their subject in the prose.
  for (const id of d.features || []) {
    text = text.replaceAll(id, m.features.get(id)?.name || "the requested feature");
  }
  if (d.rule.startsWith("execution.refused@")) text = text.replaceAll(d.subject, "This operation");
  if (brief) return text;
  const ops = warningOperations(m, d);
  const names = ops.length ? ops.map((o) => {
    const family = m.features.get(o.features?.[0])?.family;
    const count = family ? o.features.filter((id) => m.features.get(id)?.family === family).length : 0;
    return o.name + (count > 1 ? ` ×${count}` : "");
  })
    : (d.features || []).map((id) => m.features.get(id)?.name).filter(Boolean);
  return names.length ? `${[...new Set(names)].join(", ")}: ${text}` : text;
}

function linkWarning(el, m, d) {
  const key = warningOperations(m, d)[0]?.id || (d.features || []).find((id) => m.features.has(id));
  if (key) rowEvents(el, key);
}

async function showWarnings(m, notes) {
  const host = $("banner"), warned = planWarnings(m);
  host.replaceChildren();
  host.classList.remove("phone-banner-open");
  host.hidden = !warned.length && !notes.length && !m.data.error;
  if (m.data.error) {
    const error = document.createElement("div");
    error.textContent = `camlib could not run the plan: ${m.data.error}`;
    host.append(error);
  }
  for (const text of notes) {
    const line = document.createElement("div"); line.textContent = text; host.append(line);
  }
  if (!warned.length) {
    if (!host.hidden) addPhoneBannerToggle();
    return;
  }
  const rules = await loadRules();
  if (model !== m) return;
  const details = document.createElement("details"), summary = document.createElement("summary");
  details.className = "banner-warnings";
  const labels = [...new Set(warned.map((d) => rules.get(d.rule)?.warning_label || "plan needs checking"))];
  summary.textContent = `⚠ ${warned.length} warnings: ${labels.join(" · ")}`;
  details.append(summary);
  for (const d of warned) {
    const line = document.createElement("button");
    line.type = "button"; line.className = "banner-warning";
    line.textContent = warningText(m, d);
    linkWarning(line, m, d);
    details.append(line);
  }
  host.append(details);
}

function show(data) {
  setPlaying(false);
  focusKey = null; pinKey = null; time = 0; litCad = undefined;
  model = buildModel(data);
  const holes = [...model.features.values()].filter((f) => f.family === "holes").length;
  $("phone-name").textContent = data.name.replace(/\.(step|stp)$/i, "");
  $("meta").textContent = `${data.name} · ${data.analysis.binding.face_count} faces · ${holes} ${holes === 1 ? "hole" : "holes"} · ${data.plan.setups.length} ${data.plan.setups.length === 1 ? "setup" : "setups"} · ${model.lines.length} toolpath moves`;
  const notes = data.notice ? [data.notice] : [];
  const notCut = [...model.features.values()].filter((f) => f.notCut);
  if (notCut.length) {
    const names = [...new Set(notCut.map((f) => f.name.split(",")[0].toLowerCase()))].join(", ");
    notes.push(`Not cut to size: ${notCut.length} recognised ${notCut.length === 1 ? "feature" : "features"} (${names}) too narrow for the largest end mill. No smaller tool is planned for them.`);
  }
  notes.push(...data.log.filter((l) => /^note: .*no (drill|end mill)/.test(l)).map((l) => l.slice(6)));
  showWarnings(model, notes);
  showChecks(data);
  showPart(model);
  showToolpaths(model);
  buildFeatureList(model);
  layoutPanes();
  buildTimeline(model);
  buildSaveButtons(model);
  buildPlan(model);
  showStory(model);
  $("code-spacer").style.height = `${model.rows.length * ROW_HEIGHT}px`;
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
  $("start").hidden = true;
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
  count("open-step-file");
  run(file.name, await file.arrayBuffer());
}

$("file").addEventListener("change", (event) => {
  if (event.target.files[0]) {
    closePhoneMenu();
    openFile(event.target.files[0]);
  }
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
  hoveredLine = line ? line.n : null;
  drawExplain();
  setFocusQuiet(line ? line.feature || line.op : null);
});
$("code").addEventListener("pointerleave", () => {
  hoveredLine = null;
  drawExplain();
  setFocusQuiet(null);
});
const stockOn = (on) => {
  showStock = on;
  if (right.block) right.block.mesh.visible = on;
  if (on && model) drawPlayhead();
};
for (const [id, set] of [["show-rapids", (on) => { showRapids = on; }], ["show-current", (on) => { showCurrent = on; }], ["show-stock", stockOn]]) {
  $(id).addEventListener("click", () => {
    const on = $(id).getAttribute("aria-pressed") !== "true";
    $(id).setAttribute("aria-pressed", String(on));
    set(on);
    paintToolpaths();
  });
}
for (const b of document.querySelectorAll("[data-view]")) {
  b.addEventListener("click", () => viewer?.presetCamera(b.dataset.view, fitZoom(model)));
}

// Hovering the code must not scroll the code away from under the pointer.
function setFocusQuiet(key) {
  if (focusKey === key) return;
  focusKey = key;
  applyFocus(false);
}

// Three panes in the order the work happens: recognise, plan, make. Expand (or a rail, or a story
// step) opens one pane and shrinks the others to rails; on a narrow screen one is always open.
const PANES = ["features", "plan", "toolpaths"];
const NARROW = 1300;
let expanded = null;
// Move the existing controls and views, preserving handlers, state and desktop positions.
const phone = window.matchMedia("(max-width: 700px)");
const cameraButtons = document.querySelector("header > .views");
const partView = $("cad").parentElement;
const phoneMoves = new Map();
function placePhone(el, destination) {
  if (!phoneMoves.has(el)) {
    const anchor = document.createComment("desktop position");
    el.before(anchor);
    phoneMoves.set(el, anchor);
  }
  destination.append(el);
}
function closePhoneMenu() {
  $("phone-menu").close();
  $("phone-menu-button").setAttribute("aria-expanded", "false");
}
$("phone-menu-button").addEventListener("click", () => {
  $("phone-menu").showModal();
  $("phone-menu-button").setAttribute("aria-expanded", "true");
});
$("phone-menu-close").addEventListener("click", closePhoneMenu);
$("phone-menu").addEventListener("close", () => $("phone-menu-button").setAttribute("aria-expanded", "false"));
$("phone-menu-content").addEventListener("click", (event) => {
  if (event.target.closest("button")) closePhoneMenu();
});
$("phone-menu").addEventListener("click", (event) => {
  if (event.target === $("phone-menu")) closePhoneMenu();
});
for (const tab of document.querySelectorAll("[data-tab]")) {
  tab.addEventListener("click", () => expandPane(tab.dataset.tab));
  tab.addEventListener("keydown", (event) => {
    const index = PANES.indexOf(tab.dataset.tab);
    const next = event.key === "ArrowRight" ? (index + 1) % 3 : event.key === "ArrowLeft" ? (index + 2) % 3 : event.key === "Home" ? 0 : event.key === "End" ? 2 : null;
    if (next === null) return;
    event.preventDefault();
    document.querySelector(`[data-tab="${PANES[next]}"]`).focus();
    expandPane(PANES[next]);
  });
}
function layoutPhone(open) {
  if (phone.matches) {
    for (const el of [...document.querySelectorAll("header > .views, header > label.open")]) {
      if (el !== cameraButtons) placePhone(el, $("phone-menu-content"));
    }
    placePhone($("plan-save"), $("phone-menu-content"));
    placePhone(document.querySelector(".about"), $("phone-menu-content"));
    placePhone($("meta"), $("phone-meta"));
    placePhone($("banner"), $("phone-alerts"));
    placePhone($("checks"), $("phone-alerts"));
    // The part stays above Part and Plan; Make uses the linked toolpath camera.
    const view = open === "toolpaths" ? $("gcode-view") : partView;
    for (const el of [partView, $("gcode-view")]) {
      if (el !== view && phoneMoves.has(el)) phoneMoves.get(el).after(el);
    }
    placePhone(view, $("phone-view"));
    placePhone(cameraButtons, $("phone-view"));
    for (const pane of document.querySelectorAll(".pane")) {
      pane.id = "pane-" + pane.dataset.pane;
      pane.setAttribute("role", "tabpanel");
      pane.setAttribute("aria-labelledby", "tab-" + pane.dataset.pane);
    }
    for (const tab of document.querySelectorAll("[data-tab]")) {
      const selected = tab.dataset.tab === open;
      tab.setAttribute("aria-selected", String(selected));
      tab.tabIndex = selected ? 0 : -1;
    }
  } else {
    closePhoneMenu();
    for (const [el, anchor] of phoneMoves) anchor.after(el);
    for (const pane of document.querySelectorAll(".pane")) {
      pane.removeAttribute("role");
      pane.removeAttribute("aria-labelledby");
    }
    for (const step of document.querySelectorAll(".story-step")) step.removeAttribute("aria-expanded");
  }
}
function layoutPanes() {
  const panes = document.querySelector(".panes");
  const single = window.innerWidth < NARROW;
  const open = expanded || (single ? "features" : null);
  panes.classList.toggle("single", single);
  panes.style.gridTemplateColumns = open ? PANES.map((p) => (p === open ? "minmax(0, 1fr)" : "44px")).join(" ") : "";
  for (const pane of document.querySelectorAll(".pane")) pane.classList.toggle("shrunk", Boolean(open) && pane.dataset.pane !== open);
  for (const b of document.querySelectorAll(".pane-size[data-pane]")) {
    const on = b.dataset.pane === expanded;
    b.setAttribute("aria-pressed", String(on));
    b.textContent = on ? "Restore" : "Expand";
  }
  for (const b of document.querySelectorAll(".story-step")) b.setAttribute("aria-pressed", String(b.dataset.pane === open));
  layoutPhone(open || "features");
}
function expandPane(name) {
  expanded = name;
  layoutPanes();
}
for (const b of document.querySelectorAll(".pane-size[data-pane]")) {
  b.addEventListener("click", () => expandPane(expanded === b.dataset.pane ? null : b.dataset.pane));
}
for (const b of document.querySelectorAll(".pane-rail, .story-step")) {
  b.addEventListener("click", () => {
    expandPane(b.classList.contains("story-step") && expanded === b.dataset.pane && window.innerWidth >= NARROW ? null : b.dataset.pane);
    if (phone.matches && b.classList.contains("story-step")) {
      const sentence = $("story-" + b.dataset.pane).textContent;
      const hide = !$("phone-story").hidden && $("phone-story").textContent === sentence;
      $("phone-story").textContent = sentence;
      $("phone-story").hidden = hide;
      for (const step of document.querySelectorAll(".story-step")) step.setAttribute("aria-expanded", String(step === b && !hide));
    }
  });
}
window.addEventListener("resize", layoutPanes);
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && expanded && window.innerWidth >= NARROW) expandPane(null);
});
layoutPanes();

// Either view can change size on its own now, so both are watched. A hidden one is left alone.
const resized = new ResizeObserver(() => {
  if (!viewer?.ready) return;
  const box = $("cad").parentElement.getBoundingClientRect();
  if (box.width > 0 && box.height > 0) {
    viewer.resizeCadView(Math.floor(box.width), 0, Math.floor(box.height), true);
    fitCadMarker(box.width);
  }
  const view = $("gcode-view").getBoundingClientRect();
  if (view.width > 0 && view.height > 0) resizeRight();
});
resized.observe($("gcode-view"));
resized.observe($("cad").parentElement);
// The list of moves draws only the rows in view, so it redraws when its box changes (a pane opening).
new ResizeObserver(() => drawCode()).observe($("code"));

$("plan-save").addEventListener("click", savePlan);

initRight();
initCadLinks();
// Nothing is loaded until the visitor opens a file or picks a sample.
for (const b of document.querySelectorAll("[data-sample]")) {
  b.addEventListener("click", async () => {
    const name = b.dataset.sample;
    count(name === "plate.step" ? "sample-simple-plate" : "sample-complex-plate");
    try {
      const response = await fetch(name);
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      run(name, await response.arrayBuffer());
    } catch (error) {
      showBanner(`Could not load the sample ${name}: ${error.message}`);
    }
  });
}
