import { textWidth } from "./report-font.js";

// PDF diagram data in orthographic views. Coordinates remain setup-frame millimetres.
// Visibility uses bounded exact depth tests on small meshes and conservative
// raster depth on dense meshes. Uncertain classifications remain solid.
// Fill, wire and label budgets bound the retained drawing and its PDF size.
export const diagramLegend = [
  ['jaw', 'Vise jaws'], ['parallel', 'Parallels'], ['part', 'Part'],
  ['stock', 'Stock still to face'], ['stock-body', 'Stock held'], ['hidden', 'Hidden edges'], ['pickup', 'Pickup feature and zero'], ['stop', 'Stop'],
];
const identity = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
const transform = (p, s) => (s.rotation || identity).map(axis => axis.reduce((v, n, i) => v + n * (p[i] - (s.origin_model?.[i] || 0)), 0));
const pointsOf = (flat) => Array.from({ length: Math.floor(flat.length / 3) }, (_, i) => flat.slice(i * 3, i * 3 + 3));
const bounds = (points) => ({ min: [0, 1, 2].map(i => points.reduce((v, p) => Math.min(v, p[i]), Infinity)), max: [0, 1, 2].map(i => points.reduce((v, p) => Math.max(v, p[i]), -Infinity)) });
const labelFor = (pickup) => {
  if (pickup?.method === 'stock_edges') return 'Stock corner';
  // The primary face precedes seat/reference details; those may name another datum.
  const text = (pickup?.description || '').split(/[—;(]/)[0];
  const bore = text.match(/⌀[\d.]+ bore/);
  const datum = text.match(/datum [A-Z] face/);
  const face = text.match(/(?:finished )?(?:front|back|left|right|top|bottom) face/);
  return (bore?.[0] || datum?.[0] || face?.[0] || text.split(/[;(]/)[0].replace(/^the /, '')).replace(/^./, c => c.toUpperCase());
};

// Work is linear in the input mesh, but retained geometry and raster work are
// bounded. The 192-cell silhouette (128 for dense meshes) can lose details below
// one cell. B-rep wires retain their original detail within the wire budget.
// Curved-face silhouettes retain at most 2048 facing transitions per view;
// normals within 0.001 of tangent are edge-on to suppress chord-sampling noise.
function prepareMesh(mesh, setup) {
  const source = mesh?.positions || [], xyz = new Float64Array(source.length);
  const r = setup.rotation || identity, o = setup.origin_model || [0, 0, 0];
  const min = [Infinity, Infinity, Infinity], max = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < source.length; i += 3) {
    const x = source[i] - o[0], y = source[i + 1] - o[1], z = source[i + 2] - o[2];
    for (let a = 0; a < 3; a++) {
      const v = r[a][0] * x + r[a][1] * y + r[a][2] * z;
      if (!Number.isFinite(v)) throw new Error('non-finite mesh coordinates');
      xyz[i + a] = v; min[a] = Math.min(min[a], v); max[a] = Math.max(max[a], v);
    }
  }
  const faceIds = new Uint32Array((mesh?.indices?.length || 0) / 3);
  (mesh?.faces || []).forEach(([start, count], i) => faceIds.fill(i + 1, start, start + count));
  return { xyz, faceIds, part: source.length ? { min, max } : null };
}
function simplify(points, tolerance) {
  if (points.length < 3) return points;
  const distance = (p, a, b) => {
    const dx = b[0] - a[0], dy = b[1] - a[1], n = dx * dx + dy * dy;
    const t = n ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / n)) : 0;
    return (p[0] - a[0] - t * dx) ** 2 + (p[1] - a[1] - t * dy) ** 2;
  };
  const keep = new Set([0, points.length - 1]), stack = [[0, points.length - 1]];
  while (stack.length) {
    const [a, b] = stack.pop(); let best = tolerance * tolerance, at = -1;
    for (let i = a + 1; i < b; i++) { const d = distance(points[i], points[a], points[b]); if (d > best) { best = d; at = i; } }
    if (at >= 0) { keep.add(at); stack.push([a, at], [at, b]); }
  }
  return [...keep].sort((a, b) => a - b).map(i => points[i]);
}
function projection(mesh, setup, vertical, prepared) {
  const { xyz, part } = prepared, indices = mesh?.indices || [], count = indices.length / 3;
  const project = p => [p[0], p[vertical], vertical === 1 ? p[2] : -p[1]];
  if (!part) return { part, project, loops: [], lines: [], stats: { triangles: 0, samples: 0 } };
  const n = count > 20000 ? 128 : 192, stride = n + 1;
  const x0 = part.min[0], y0 = part.min[vertical];
  const sx = Math.max(part.max[0] - x0, 1e-6) / n, sy = Math.max(part.max[vertical] - y0, 1e-6) / n;
  const span = Math.max(part.max[0] - x0, part.max[vertical] - y0, 1);
  const depth = new Float64Array(n * n).fill(-Infinity), triangles = [];
  const signs = new Int8Array(count), faceMasks = new Uint8Array((mesh?.faces?.length || 0) + 1);
  let rasterTests = 0, rasterRows = 0;
  for (let i = 0; i < indices.length; i += 3) {
    const ia = indices[i] * 3, ib = indices[i + 1] * 3, ic = indices[i + 2] * 3;
    const ax = xyz[ia], ay = xyz[ia + vertical], bx = xyz[ib], by = xyz[ib + vertical], cx = xyz[ic], cy = xyz[ic + vertical];
    const az = vertical === 1 ? xyz[ia + 2] : -xyz[ia + 1], bz = vertical === 1 ? xyz[ib + 2] : -xyz[ib + 1], cz = vertical === 1 ? xyz[ic + 2] : -xyz[ic + 1];
    const area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
    const normalLength = Math.hypot((by - ay) * (cz - az) - (bz - az) * (cy - ay), (bz - az) * (cx - ax) - (bx - ax) * (cz - az), area);
    // Chord sampling at opposite ends of a cylinder can stagger by a few
    // microns. Treat near-tangent normals as edge-on, not a false facing flip.
    const tangent = Math.max(1e-12, normalLength * 0.001);
    const sign = area > tangent ? 1 : area < -tangent ? -1 : 0;
    signs[i / 3] = sign; faceMasks[prepared.faceIds[i / 3]] |= sign > 0 ? 1 : sign < 0 ? 2 : 0;
    if (Math.abs(area) < 1e-12) continue;
    if (count <= 20000) triangles.push([ax, ay, az, bx, by, bz, cx, cy, cz, area]);
    const lx = Math.max(0, Math.ceil((Math.min(ax, bx, cx) - x0) / sx - 0.5)), hx = Math.min(n - 1, Math.floor((Math.max(ax, bx, cx) - x0) / sx - 0.5));
    const ly = Math.max(0, Math.ceil((Math.min(ay, by, cy) - y0) / sy - 0.5)), hy = Math.min(n - 1, Math.floor((Math.max(ay, by, cy) - y0) / sy - 0.5));
    if (lx > hx || ly > hy) continue;
    // Scan-convert the actual horizontal span, not the triangle's bounding
    // rectangle. Fan ends and long thin triangles cost only their covered area.
    for (let y = ly; y <= hy; y++) {
      if (++rasterRows > 4000000) throw new Error('mesh projection exceeds raster budget');
      const py = y0 + (y + 0.5) * sy;
      let left = Infinity, right = -Infinity;
      const intersect = (x1, y1, x2, y2) => {
        if (py < Math.min(y1, y2) - 1e-9 || py > Math.max(y1, y2) + 1e-9) return;
        if (Math.abs(y2 - y1) < 1e-12) { left = Math.min(left, x1, x2); right = Math.max(right, x1, x2); }
        else { const x = x1 + (py - y1) * (x2 - x1) / (y2 - y1); left = Math.min(left, x); right = Math.max(right, x); }
      };
      intersect(ax, ay, bx, by); intersect(bx, by, cx, cy); intersect(cx, cy, ax, ay);
      const start = Math.max(lx, Math.ceil((left - x0) / sx - 0.5 - 1e-7)), end = Math.min(hx, Math.floor((right - x0) / sx - 0.5 + 1e-7));
      rasterTests += Math.max(0, end - start + 1);
      if (rasterTests > 4000000) throw new Error('mesh projection exceeds raster budget');
      for (let x = start; x <= end; x++) {
        const px = x0 + (x + 0.5) * sx;
        const u = ((px - ax) * (cy - ay) - (py - ay) * (cx - ax)) / area;
        const v = ((bx - ax) * (py - ay) - (by - ay) * (px - ax)) / area;
        if (u >= -1e-7 && v >= -1e-7 && u + v <= 1 + 1e-7) depth[y * n + x] = Math.max(depth[y * n + x], az + u * (bz - az) + v * (cz - az));
      }
    }
  }

  // Trace the raster union into closed outer/inner loops; one even-odd fill
  // preserves holes without retaining a polygon for every input triangle.
  const boundary = new Map(); let edgeCount = 0;
  const edge = (a, b) => { if (!boundary.has(a)) boundary.set(a, []); boundary.get(a).push(b); edgeCount++; };
  const filled = (x, y) => x >= 0 && x < n && y >= 0 && y < n && depth[y * n + x] !== -Infinity;
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) if (filled(x, y)) {
    const a = y * stride + x;
    if (!filled(x, y - 1)) edge(a, a + 1);
    if (!filled(x + 1, y)) edge(a + 1, a + 1 + stride);
    if (!filled(x, y + 1)) edge(a + 1 + stride, a + stride);
    if (!filled(x - 1, y)) edge(a + stride, a);
  }
  if (edgeCount > 20000) throw new Error('projected outline exceeds contour budget');
  const loops = []; let vertices = 0;
  while (boundary.size) {
    const first = boundary.keys().next().value, points = []; let at = first;
    do {
      points.push([x0 + (at % stride) * sx, y0 + Math.floor(at / stride) * sy]);
      const next = boundary.get(at); if (!next?.length) throw new Error('open projected contour');
      const old = at; at = next.pop(); if (!next.length) boundary.delete(old);
    } while (at !== first);
    points.push(points[0]);
    const loop = simplify(points, Math.min(sx, sy) * 0.65); vertices += loop.length;
    if (vertices > 8000) throw new Error('projected outline exceeds vertex budget');
    loops.push(loop);
  }
  const bins = Array.from({ length: 1024 }, () => []), cell = span / 32;
  const bin = (v, lo) => Math.max(0, Math.min(31, Math.floor((v - lo) / cell)));
  let refs = 0, complete = count <= 20000, samples = 0;
  if (complete) for (const t of triangles) {
    const lx = bin(Math.min(t[0], t[3], t[6]), x0), hx = bin(Math.max(t[0], t[3], t[6]), x0);
    const ly = bin(Math.min(t[1], t[4], t[7]), y0), hy = bin(Math.max(t[1], t[4], t[7]), y0);
    refs += (hx - lx + 1) * (hy - ly + 1);
    if (refs > 1000000) { complete = false; break; }
    for (let y = ly; y <= hy; y++) for (let x = lx; x <= hx; x++) bins[y * 32 + x].push(t);
  }
  function hidden(p) {
    if (samples++ >= 20000) return false;
    if (!complete) {
      // Dense meshes use a conservative depth neighbourhood: uncertain edges
      // stay solid, including gaps and features below raster resolution.
      const ix = Math.floor((p[0] - x0) / sx), iy = Math.floor((p[1] - y0) / sy);
      let nearest = Infinity;
      for (let y = iy - 1; y <= iy + 1; y++) for (let x = ix - 1; x <= ix + 1; x++) {
        if (!filled(x, y)) return false; nearest = Math.min(nearest, depth[y * n + x]);
      }
      return nearest > p[2] + Math.max(sx, sy);
    }
    const candidates = bins[bin(p[1], y0) * 32 + bin(p[0], x0)];
    if (candidates.length > 256) return false;
    for (const t of candidates) {
      const u = ((p[0] - t[0]) * (t[7] - t[1]) - (p[1] - t[1]) * (t[6] - t[0])) / t[9];
      const v = ((t[3] - t[0]) * (p[1] - t[1]) - (t[4] - t[1]) * (p[0] - t[0])) / t[9];
      if (u >= -1e-7 && v >= -1e-7 && u + v <= 1 + 1e-7 && t[2] + u * (t[5] - t[2]) + v * (t[8] - t[2]) > p[2] + span * 1e-5) return true;
    }
    return false;
  }
  // Indexed vertex incidence avoids the former unbounded string-keyed edge
  // map. Only faces with both facing signs can have interior silhouettes.
  const curves = new Map(); let comparisons = 0, silhouetteEdges = 0, silhouetteTruncated = false;
  if (faceMasks.some(mask => mask === 3)) {
    if (!prepared.incidence) {
      const canonical = new Uint32Array(xyz.length / 3), weld = new Map(), epsilon = span * 1e-7;
      for (let v = 0; v < canonical.length; v++) {
        if (count > 20000) { canonical[v] = v; continue; }
        const key = [0, 1, 2].map(a => Math.round(xyz[v * 3 + a] / epsilon)).join(',');
        if (!weld.has(key)) weld.set(key, v); canonical[v] = weld.get(key);
      }
      const heads = new Int32Array(xyz.length / 3).fill(-1), next = new Int32Array(indices.length);
      for (let i = 0; i < indices.length; i++) { const v = canonical[indices[i]]; next[i] = heads[v]; heads[v] = i; }
      prepared.incidence = { heads, next, canonical };
    }
    const { heads, next, canonical } = prepared.incidence;
    outer: for (let t = 0; t < count; t++) {
      const face = prepared.faceIds[t];
      if (faceMasks[face] !== 3 || signs[t] < 0) continue;
      for (let e = 0; e < 3; e++) {
        const a = canonical[indices[t * 3 + e]], b = canonical[indices[t * 3 + (e + 1) % 3]]; let degree = 0;
        for (let link = heads[a]; link >= 0 && degree++ < 64; link = next[link]) {
          if (++comparisons > 4000000 || silhouetteEdges >= 2048) { silhouetteTruncated = true; break outer; }
          const other = Math.floor(link / 3);
          if (other === t || prepared.faceIds[other] !== face || signs[other] >= signs[t] || (signs[t] === 0 && signs[other] === 0)) continue;
          const j = other * 3;
          if (canonical[indices[j]] !== b && canonical[indices[j + 1]] !== b && canonical[indices[j + 2]] !== b) continue;
          if (!curves.has(face)) curves.set(face, []);
          curves.get(face).push([a, b]); silhouetteEdges++; break;
        }
      }
    }
  }
  const lines = []; let wireSegments = 0, wireVertices = 0;
  function drawWire(points, source, face) {
    let run = [], state;
    const emit = () => {
      const p = simplify(run, span * 1e-8); wireVertices += p.length;
      if (wireVertices > 8000) return false;
      lines.push({ type: 'polyline', class: state ? 'hidden' : 'edge', points: p, source, ...(face == null ? {} : { face }) }); return true;
    };
    for (let j = 1; j < points.length; j++) {
      const a = points[j - 1], b = points[j]; wireSegments++;
      const length = Math.hypot(b[0] - a[0], b[1] - a[1]); if (length < span * 1e-8) continue;
      const steps = samples >= 20000 ? 1 : Math.min(64, Math.max(1, Math.ceil(length / (span / 160))));
      let last = a.slice(0, 2); if (!run.length) run.push(last);
      for (let k = 0; k < steps; k++) {
        const at = f => a.map((v, i) => v + (b[i] - v) * f), h = hidden(at((k + 0.5) / steps));
        if (state !== undefined && h !== state) { if (!emit()) return false; run = [last]; }
        state = h; last = at((k + 1) / steps).slice(0, 2); run.push(last);
      }
    }
    return run.length <= 1 || emit();
  }
  for (const flat of mesh?.edges || []) {
    if (wireSegments + flat.length / 3 > 12000) break;
    if (!drawWire(pointsOf(flat).map(p => project(transform(p, setup))), 'brep')) break;
  }
  // Join adjacent silhouette edges before classification and PDF output. Long
  // hole walls stay continuous, including hidden dashes and tessellated curves.
  for (const [face, edges] of curves) {
    const links = new Map(), used = new Set();
    edges.forEach(([a, b], i) => { for (const v of [a, b]) { if (!links.has(v)) links.set(v, []); links.get(v).push(i); } });
    for (let i = 0; i < edges.length; i++) if (!used.has(i)) {
      const [a, b] = edges[i]; let at = links.get(a).length === 1 ? a : links.get(b).length === 1 ? b : a;
      const chain = [at];
      while (true) {
        const e = links.get(at).find(j => !used.has(j)); if (e == null) break;
        used.add(e); at = edges[e][0] === at ? edges[e][1] : edges[e][0]; chain.push(at);
      }
      if (!drawWire(chain.map(v => project([xyz[v * 3], xyz[v * 3 + 1], xyz[v * 3 + 2]])), 'silhouette', face - 1)) { silhouetteTruncated = true; break; }
    }
  }

  return { part, project, loops, lines, stats: { triangles: count, samples: Math.min(samples, 20000), rasterTests, rasterRows, silhouetteEdges, silhouetteTruncated, comparisons, cell_mm: Math.max(sx, sy), contourVertices: vertices } };
}

// Conservative DejaVu Sans bounds, in setup mm (baseline + ascenders/descenders).
function diagramTextBox(shape) {
  const height = shape.fontSize, width = textWidth(shape.text, height);
  const x = shape.x - (shape.anchor === 'middle' ? width / 2 : shape.anchor === 'end' ? width : 0);
  return [x, shape.y - height * 0.3, x + width, shape.y + height * 1.1];
}
const overlaps = (a, b) => a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1];
function crossesBox(a, b, box) {
  let lo = 0, hi = 1;
  for (let axis = 0; axis < 2; axis++) {
    const delta = b[axis] - a[axis];
    if (Math.abs(delta) < 1e-12) { if (a[axis] < box[axis] || a[axis] > box[axis + 2]) return false; }
    else {
      const t = [(box[axis] - a[axis]) / delta, (box[axis + 2] - a[axis]) / delta].sort((a, b) => a - b);
      lo = Math.max(lo, t[0]); hi = Math.min(hi, t[1]);
      if (lo > hi) return false;
    }
  }
  return true;
}
// Shared pickup origins may meet; crossing or overlapping leader runs may not.
function leadersCross(a, b, c, d, clearance) {
  const cross = (u, v) => u[0] * v[1] - u[1] * v[0];
  const sub = (u, v) => u.map((n, i) => n - v[i]);
  const r = sub(b, a), t = sub(d, c), offset = sub(c, a), den = cross(r, t), epsilon = 1e-8;
  if (Math.abs(den) < epsilon) {
    if (Math.abs(cross(offset, r)) <= epsilon) {
      const axis = Math.abs(r[0]) > Math.abs(r[1]) ? 0 : 1;
      if (Math.min(Math.max(a[axis], b[axis]), Math.max(c[axis], d[axis])) - Math.max(Math.min(a[axis], b[axis]), Math.min(c[axis], d[axis])) > epsilon) return true;
    }
  } else {
    const u = cross(offset, t) / den, v = cross(offset, r) / den;
    if (u >= -epsilon && u <= 1 + epsilon && v >= -epsilon && v <= 1 + epsilon && !(Math.abs(u) < epsilon && Math.abs(v) < epsilon)) return true;
  }
  if (Math.hypot(...offset) < Math.max(epsilon, clearance)) return false;
  const distance = (p, a, b) => {
    const delta = sub(b, a), length = delta[0] ** 2 + delta[1] ** 2;
    const u = length ? Math.max(0, Math.min(1, sub(p, a).reduce((n, v, i) => n + v * delta[i], 0) / length)) : 0;
    return Math.hypot(...p.map((v, i) => v - a[i] - u * delta[i]));
  };
  return Math.min(distance(a,c,d),distance(b,c,d),distance(c,a,b),distance(d,a,b)) < clearance;
}

function layoutLabels(shapes, annotations, unit, part) {
  const expand = box => box.map((v, i) => v + (i < 2 ? -1 : 1) * unit);
  const segments = s => s.points.slice(1).map((p, i) => [s.points[i], p]);
  const obstacles = shapes.filter(s => ['stock', 'stock-body', 'dimension', 'axis', 'jaw', 'jaw-near', 'parallel', 'part', 'edge', 'hidden', 'pickup'].includes(s.class) && s.type !== 'text').flatMap(s => s.type === 'rect'
    ? [[[s.x, s.y], [s.x + s.width, s.y]], [[s.x, s.y + s.height], [s.x + s.width, s.y + s.height]], [[s.x, s.y], [s.x, s.y + s.height]], [[s.x + s.width, s.y], [s.x + s.width, s.y + s.height]]]
    : s.type === 'outline' ? s.loops.flatMap(points => segments({ points })) : segments(s));
  const blocks = shapes.filter(s => s.class === 'stop' || s.class === 'zero').map(s => s.type === 'rect' ? [s.x, s.y, s.x + s.width, s.y + s.height] : [s.x - s.radius, s.y - s.radius, s.x + s.radius, s.y + s.radius]);
  const labels = shapes.filter(s => s.type === 'text'), placed = [], leaders = [];
  // Reserve space beside dimensions first, then fixed captions and pickups.
  // Later leaders route around both the captions and earlier leaders.
  const priority = s => s.text.startsWith('Stop') ? 0 : s.text.startsWith('Datum') ? 1 : 2;
  const ordered = [...labels.filter(s => s.dimension), ...labels.filter(s => !annotations.some(a => a.shape === s)), ...annotations.map(a => a.shape).filter(s => !s.dimension).sort((a, b) => priority(a) - priority(b))];
  const partInterior = part && [part.min[0] + unit / 10, part.min[1] + unit / 10, part.max[0] - unit / 10, part.max[1] - unit / 10];
  const jawBoxes = shapes.filter(s => ['jaw', 'jaw-near'].includes(s.class)).map(s => [s.x, s.y, s.x + s.width, s.y + s.height]);
  const contains = (box, p) => p[0] >= box[0] - 1e-8 && p[0] <= box[2] + 1e-8 && p[1] >= box[1] - 1e-8 && p[1] <= box[3] + 1e-8;
  for (const shape of ordered) {
    const annotation = annotations.find(a => a.shape === shape), initial = [shape.x, shape.y, shape.anchor], candidates = [];
    const offsets = shape.dimension ? [0, -0.5, 0.5, -1, 1, -1.5, 1.5, -2, 2, -3, 3, -4, 4, -6, 6, -8, 8, -16, 16] : annotation ? [0, -4, 4, -8, 8, -16, 16, -32, 32, -48, 48] : [0, -8, 8, -16, 16];
    // Search positions before choosing a route. A free nearby straight leader
    // wins over an elbow from the first free (but badly placed) caption box.
    for (let row = 0; row < 64; row++) for (const dx of offsets) for (const anchor of annotation ? [initial[2], ...['start', 'middle', 'end'].filter(a => a !== initial[2])] : [initial[2]]) {
      shape.x = initial[0] + dx * unit;
      shape.y = initial[1] + (row ? Math.ceil(row / 2) * (row % 2 ? 1 : -1) * (shape.dimension ? 0.5 : 6) : 0) * unit; shape.anchor = anchor;
      const textBox = diagramTextBox(shape), box = expand(textBox);
      let end, distance, captionDistance;
      if (annotation) {
        if (contains(box, annotation.from)) continue;
        captionDistance = Math.hypot(Math.max(textBox[0] - annotation.from[0], 0, annotation.from[0] - textBox[2]), Math.max(textBox[1] - annotation.from[1], 0, annotation.from[1] - textBox[3]));
        end = [Math.max(box[0], Math.min(box[2], annotation.from[0])), Math.max(box[1], Math.min(box[3], annotation.from[1]))];
        const d = Math.hypot(end[0] - annotation.from[0], end[1] - annotation.from[1]);
        end = end.map((v, i) => v + Math.min(unit, d / 2) * (annotation.from[i] - v) / d);
        distance = Math.hypot(end[0] - annotation.from[0], end[1] - annotation.from[1]);
      } else distance = Math.hypot(shape.x - initial[0], shape.y - initial[1]);
      candidates.push({ x: shape.x, y: shape.y, anchor, box, end, distance, captionDistance });
    }
    candidates.sort((a, b) => a.distance - b.distance);
    let fitted = false;
    for (const elbows of annotation ? [false, true] : [false]) {
      for (const candidate of candidates) {
        const { box, end } = candidate;
        // Keep dimension captions within 3 mm on paper of their own midpoint.
        if (shape.dimension && candidate.captionDistance > unit * (3 / 0.6)) continue;
        if (part && (annotation || shape.class === 'tag') && overlaps(box, [part.min[0], part.min[1], part.max[0], part.max[1]])) continue;
        if (placed.some(b => overlaps(box, b)) || blocks.some(b => overlaps(box, b)) || [...obstacles, ...leaders].some(([a, b]) => crossesBox(a, b, box))) continue;
        if (annotations.some(a => a !== annotation && contains(box, a.from))) continue;
        let route;
        if (annotation) {
          const paths = [[annotation.from, end]];
          if (elbows) {
            paths.push([annotation.from, [end[0], annotation.from[1]], end], [annotation.from, [annotation.from[0], end[1]], end]);
            for (const rect of placed) for (const rail of [rect[0] - unit, rect[2] + unit]) paths.push([annotation.from, [rail, annotation.from[1]], [rail, end[1]], end]);
          }
          // Leave room to route later leaders out of their pickup anchors.
          const anchorClear = (a, b) => annotations.every(other => other === annotation
            || other.from.every((v, i) => Math.abs(v - annotation.from[i]) <= unit / 2)
            || !crossesBox(a, b, [other.from[0] - unit / 2, other.from[1] - unit / 2, other.from[0] + unit / 2, other.from[1] + unit / 2]));
          const valid = paths.filter(points => segments({ points }).every(([a, b]) =>
            !leaders.some(([c, d]) => leadersCross(a, b, c, d, unit / 2)) && anchorClear(a, b)
            && (!partInterior || contains(partInterior, annotation.from) || !crossesBox(a, b, partInterior))
            && [...placed, box].every(rect => !crossesBox(a, b, rect))
            && jawBoxes.every(rect => contains(rect, annotation.from) || !crossesBox(a, b, rect))));
          const length = p => segments({ points: p }).reduce((n, [a, b]) => n + Math.hypot(b[0] - a[0], b[1] - a[1]), 0);
          valid.sort((a, b) => length(a) - length(b)); route = valid[0];
          if (!route) continue;
        }
        shape.x = candidate.x; shape.y = candidate.y; shape.anchor = candidate.anchor;
        placed.push(box);
        if (route) { annotation.leader.points = route; leaders.push(...segments({ points: route })); }
        fitted = true; break;
      }
      if (fitted) break;
    }
    if (!fitted) throw new Error(`Cannot place setup diagram label: ${shape.text}`);
  }

}

function setupDiagram(data, setup, format = String) {
  const c = setup.clamping || {}, stock = setup.stock;
  const zero = setup.zero === 'stock' ? { status: 'located', xy: { method: 'stock_edges', description: 'Stock corner', features: [] }, z: { description: 'Stock top', features: [] } } : setup.zero;
  const prepared = prepareMesh(data.mesh, setup);
  const facing = setup.operations?.find(o => o.kind === 'facing' && Number.isFinite(o.floor_z));
  const heldStock = setup === data.plan?.setups?.[0] || setup.zero === 'stock' || zero?.xy?.method === 'stock_edges';
  const number = value => String(format(value)).replace(/-/g, '−');
  const located = zero?.status === 'located';
  const warnings = located ? [] : [zero?.status === 'warning' ? 'Pickup warning: see Zero before locating this setup.' : 'Pickup warning: no recorded setup zero.'];
  const views = [1, 2].map(vertical => {
    const geometry = projection(data.mesh, setup, vertical, prepared), shapes = [];
    const part = geometry.part || stock || { min: [0, 0, -1], max: [1, 1, 0] };
    const size = Math.max(part.max[0] - part.min[0], part.max[vertical] - part.min[vertical], 20), unit = size / 100;
    const add = (type, cls, rest) => { const shape = { type, class: cls, ...rest }; shapes.push(shape); return shape; };
    const line = (cls, a, b) => add('polyline', cls, { points: [a, b] });
    const text = (cls, x, y, text, anchor = 'start') => add('text', cls, { x, y, text: text.replace(/-(?=\d)/g, '−'), anchor, fontSize: 3.4 * unit });
    const annotations = [];
    const annotation = (from, x, y, caption, anchor = 'start', cls = 'pickup') => {
      const leader = line(cls === 'dimension' ? 'guide' : 'leader', from, [x, y]);
      const shape = text(cls, x, y, caption, anchor);
      annotations.push({ shape, from, leader });
      return shape;
    };
    const rect = (cls, box) => add('rect', cls, { x: box.min[0], y: box.min[vertical], width: box.max[0] - box.min[0], height: box.max[vertical] - box.min[vertical] });
    let cropped = false;
    const jaws = (c.jaws || []).map(j => {
      const box = { min: [...j.min], max: [...j.max] };
      if (vertical === 1 && c.faces?.[0]?.includes('.x.')) {
        const along = 1;
        box.min[along] = Math.max(box.min[along], part.min[along] - 14 * unit);
        box.max[along] = Math.min(box.max[along], part.max[along] + 14 * unit);
        cropped ||= box.min[along] !== j.min[along] || box.max[along] !== j.max[along];
      }
      return box;
    });
    if (vertical === 2 && jaws.length) {
      const bed = Math.min(...jaws.map(j => j.min[2]));
      add('rect', 'bed', { x: Math.min(...jaws.map(j => j.min[0])) - 4 * unit, y: bed - 4 * unit, width: Math.max(...jaws.map(j => j.max[0])) - Math.min(...jaws.map(j => j.min[0])) + 8 * unit, height: 4 * unit });
    }
    const alongY = vertical === 2 && (c.fixed_jaw?.startsWith('y') || c.faces?.[0]?.includes('.y.'));
    const nearIndex = alongY ? (c.jaws || []).reduce((best, j, i, all) => j.min[1] < all[best].min[1] ? i : best, 0) : -1;
    jaws.forEach((jaw, i) => { if (i !== nearIndex) rect('jaw', jaw); });
    for (const p of c.parallels || []) rect('parallel', p);
    if (heldStock && stock) rect('stock-body', stock);
    if (geometry.loops.length) add('outline', 'part', { loops: geometry.loops });
    shapes.push(...geometry.lines);
    if (!geometry.part) text('tag', part.min[0], part.max[vertical] + 5 * unit, 'Part mesh unavailable');
    // A facing allowance comes from its operation, never envelope slack.
    if (stock && facing && stock.max[2] > facing.floor_z) {
      if (vertical === 1) rect('stock', stock);
      else rect('stock', { min: [stock.min[0], 0, facing.floor_z], max: stock.max });
    }
    // The jaw nearest a +Y viewer is in front of the material. Paint its outline last.
    if (nearIndex >= 0) rect('jaw-near', jaws[nearIndex]);
    const jawAxis = c.fixed_jaw?.startsWith('x') ? 0 : 1;
    const sorted = (c.jaws || []).map((j, i) => ({ j, i })).sort((a, b) => a.j.min[jawAxis] - b.j.min[jawAxis]);
    const fixedIndex = c.fixed_jaw ? sorted[c.fixed_jaw.endsWith('min') ? 0 : sorted.length - 1]?.i : -1;
    jaws.forEach((j, i) => {
      const label = i === fixedIndex ? 'Fixed jaw' : fixedIndex < 0 ? 'Jaw (fixed not recorded)' : 'Moving jaw';
      if (alongY) { if (i === nearIndex) text('tag', (j.min[0] + j.max[0]) / 2, j.min[2] + 5 * unit, `${label} (near side)`, 'middle'); }
      else text('tag', (j.min[0] + j.max[0]) / 2, vertical === 1 ? j.max[1] - 4 * unit : j.min[2] + 5 * unit, label, 'middle');
    });
    function dimension(x, low, high, caption, value, anchor = 'start') {
      if (!Number.isFinite(value) || high <= low) return;
      line('dimension', [x, low], [x, high]);
      for (const y of [low, high]) line('dimension', [x - 1.6 * unit, y], [x + 1.6 * unit, y]);
      const label = annotation([x, (low + high) / 2], x + (anchor === 'end' ? -4 : 4) * unit, (low + high) / 2, `${caption} ${number(value)}`, anchor, 'dimension');
      label.dimension = { kind: caption, value, low, high };
    }
    if (vertical === 2) {
      // Put support dimensions outside the held material and jaws so captions
      // can stay beside their lines, even when a short grip cannot fit text.
      const left = Math.min(part.min[0], stock?.min[0] ?? part.min[0], ...jaws.map(j => j.min[0]), ...(c.parallels || []).map(p => p.min[0]));
      const right = Math.max(part.max[0], stock?.max[0] ?? part.max[0], ...jaws.map(j => j.max[0]), ...(c.parallels || []).map(p => p.max[0]));
      const p = c.parallels?.[0];
      if (p) dimension(right + 6 * unit, p.min[2], p.max[2], 'Parallels', p.max[2] - p.min[2]);
      const seat = c.jaw_top_z - c.grip_height_mm;
      if (c.grip_height_mm != null) dimension(left - 6 * unit, seat, c.jaw_top_z, 'Grip', c.grip_height_mm, 'end');
      if (stock && facing) dimension(part.max[0] + 8 * unit, facing.floor_z, stock.max[2], 'Facing', stock.max[2] - facing.floor_z);
    }
    if (located) {
      const r = 2.6 * unit;
      line('axis', [0, 0], [14 * unit, 0]); line('axis', [0, 0], [0, 14 * unit]);
      add('circle', 'zero', { x: 0, y: 0, radius: r });
      for (const start of [0, Math.PI]) add('polygon', 'zero-fill', { points: [[0, 0], ...Array.from({ length: 9 }, (_, i) => [r * Math.cos(start + i * Math.PI / 16), r * Math.sin(start + i * Math.PI / 16)])] });
      text('pickup', 15 * unit, 3.5 * unit, 'X'); text('pickup', 0, 18 * unit, vertical === 1 ? 'Y' : 'Z');
      annotation([0, 0], -8 * unit, -8 * unit, vertical === 1 ? 'X0 Y0' : 'Z0 here', 'end');
      if (vertical === 1 && !zero.xy?.axes && !zero.xy?.features?.length) annotation([0, 0], 8 * unit, -12 * unit, labelFor(zero.xy));
      const pickups = [zero.xy, ...Object.values(zero.xy?.axes || {}), ...(vertical === 2 ? [zero.z] : [])];
      const ids = new Set(pickups.flatMap(p => p?.target_features?.length ? p.target_features : p?.features || []));

      // Highlight the actual mesh boundary of each picked face, not its box.
      for (const id of ids) {
        const faceIds = id.startsWith('faces:') ? id.slice(6).split('.').map(Number) : data.analysis?.features?.find(f => f.id === id)?.faces || [];
        for (const face of faceIds) {
          const range = data.mesh?.faces?.[face];
          if (!range) continue;
          const record = data.analysis?.faces?.find(f => f.id === face);
          const normal = (setup.rotation || identity).map(axis => axis.reduce((v, n, i) => v + n * (record?.direction?.[i] || 0), 0));
          if ((record?.kind === 'plane' || (!record?.kind && id.startsWith('faces:'))) && Math.abs(normal[vertical === 1 ? 2 : 1]) > 0.999) continue;
          const boundary = new Map();
          if (range[1] > 12000) continue;
          for (let n = range[0] * 3; n < (range[0] + range[1]) * 3; n += 3) {
            const indices = data.mesh.indices.slice(n, n + 3);
            for (let i = 0; i < 3; i++) {
              const a = indices[i], b = indices[(i + 1) % 3], key = [a, b].sort((x, y) => x - y).join(':');
              if (boundary.has(key)) boundary.delete(key); else boundary.set(key, [a, b]);
            }
          }
          for (const pair of boundary.values()) {
            const p = pair.map(i => geometry.project(transform(data.mesh.positions.slice(i * 3, i * 3 + 3), setup)).slice(0, 2));
            if (Math.hypot(p[1][0] - p[0][0], p[1][1] - p[0][1]) > 1e-7) add('polyline', 'pickup', { points: p, feature: id });
          }
          if (record?.centroid) {
            const p = geometry.project(transform(record.centroid, setup));
            const pickup = [...Object.values(zero.xy?.axes || {}), zero.z, zero.xy].find(p => p?.features?.includes(id) || p?.target_features?.includes(id));
            const caption = labelFor(pickup) || `Face ${face}`;
            const nx = normal[0], ny = normal[vertical];
            let tx = p[0], ty = part.max[vertical] + 12 * unit, anchor = 'middle';
            let from = p.slice(0, 2);
            if (Math.abs(nx) > Math.abs(ny) && Math.abs(nx) > 0.1) {
              tx = (nx < 0 ? part.min[0] - 8 * unit : part.max[0] + 8 * unit);
              ty = p[1] + 5 * unit; anchor = nx < 0 ? 'end' : 'start'; from = p.slice(0, 2);
            } else if (Math.abs(ny) > 0.1) {
              ty = (ny < 0 ? part.min[vertical] - 18 * unit : part.max[vertical] + 12 * unit);
              from = p.slice(0, 2);
            }
            annotation(from, tx, ty, caption, anchor);
          }
        }
      }
      const stop = c.part_stop;
      if (stop) {
        const contact = zero.xy?.axes?.[stop.axis]?.contact_in_previous_frame_mm;
        const p = contact ? contact.map((v, i) => v - (zero.offset_in_previous_frame_mm?.[i] || 0)) : [0, 0, stop.contact_z_mm];
        p[stop.axis === 'x' ? 0 : 1] = stop.coordinate_mm; p[2] = stop.contact_z_mm;
        const q = geometry.project(p), top = stop.maximum_z_mm ?? stop.contact_z_mm;
        const axis = stop.axis === 'x' ? 0 : 1;
        const negativeSide = stop.coordinate_mm <= (part.min[axis] + part.max[axis]) / 2;
        const along = negativeSide ? 6 : 0, height = vertical === 2 ? Math.max(6 * unit, top - stop.contact_z_mm) : 6 * unit;
        add('rect', 'stop', { x: q[0] - (axis === 0 ? along : 3) * unit, y: vertical === 1 ? q[1] - (axis === 1 ? along : 3) * unit : top - height, width: 6 * unit, height, contact: p, maximum_z_mm: stop.maximum_z_mm });
        annotation(q.slice(0, 2), q[0] + 5 * unit, vertical === 1 ? q[1] - 12 * unit : top + 9 * unit, `Stop${stop.maximum_z_mm == null ? '' : ` · nose ≤ Z${number(stop.maximum_z_mm)}`}`);
      }
    }
    if (shapes.length > 4000 || shapes.reduce((n, s) => n + (s.points?.length || s.loops?.reduce((v, loop) => v + loop.length, 0) || 0), 0) > 20000) throw new Error('drawing exceeds geometry budget');
    // Geometry stays in setup mm. The page scale is explicit and text is laid
    // out afresh at 7.5 pt at that scale; the renderer must never shrink it.
    const geometryPoints = shapes.filter(s => s.type !== 'text' && s.class !== 'leader').flatMap(s => s.points || s.loops?.flat() || (s.type === 'rect' ? [[s.x, s.y], [s.x + s.width, s.y + s.height]] : [[s.x, s.y]]));
    const gb = bounds(geometryPoints.map(p => [...p, 0]));
    let scale = Math.min(120 / Math.max(1, gb.max[0] - gb.min[0]), 48 / Math.max(1, gb.max[1] - gb.min[1])), viewBounds;
    const initial = shapes.filter(s => s.type === 'text').map(s => [s, s.x, s.y]);
    for (let attempt = 0; attempt < 10; attempt++) {
      for (const [s, x, y] of initial) { s.x = x; s.y = y; s.fontSize = 7.5 * 0.352778 / scale; s.fontPt = 7.5; }
      layoutLabels(shapes, annotations, 0.6 / scale, geometry.part ? { min: [part.min[0], part.min[vertical]], max: [part.max[0], part.max[vertical]] } : null);
      const extents = shapes.flatMap(s => s.points || s.loops?.flat() || (s.type === 'rect' ? [[s.x, s.y], [s.x + s.width, s.y + s.height]] : s.type === 'text' ? [diagramTextBox(s).slice(0, 2), diagramTextBox(s).slice(2)] : [[s.x, s.y]]));
      const b = bounds(extents.map(p => [...p, 0])), pad = 2 / scale;
      viewBounds = [b.min[0] - pad, b.min[1] - pad, b.max[0] + pad, b.max[1] + pad];
      const w = (viewBounds[2] - viewBounds[0]) * scale, h = (viewBounds[3] - viewBounds[1]) * scale;
      if (w <= 180 && h <= 95) break;
      if (attempt === 9) throw new Error('labels exceed printable diagram area');
      scale *= Math.min(175 / w, 90 / h, 0.9);
    }
    return { caption: vertical === 1 ? 'Seen from above (machine Z down)' : 'Seen from the front (looking +Y)', shapes, unit, bounds: viewBounds, scale, height: (viewBounds[3] - viewBounds[1]) * scale, partBounds: part, cropped, note: geometry.stats.silhouetteTruncated ? 'Some curved edges omitted' : '', stats: geometry.stats };

  });
  return { type: 'setup-diagram', setup: setup.id, views, warnings, legend: diagramLegend.filter(([cls]) => views.some(v => v.shapes.some(s => s.class === cls || (cls === 'jaw' && s.class === 'jaw-near')))), note: views[0].cropped ? 'Jaws cropped in Y in the view from above.' : '' };
}

// One unavailable drawing must never prevent the rest of the setup sheet.
export function buildSetupDiagram(data, setup, format = String) {
  try { return setupDiagram(data, setup, format); }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const limited = /exceeds? .*budget|exceeds? .*area/.test(message);
    if (!limited) console.warn('Setup diagram not drawn', error);
    const reason = limited ? 'the part is too detailed to draw here.'
      : message === 'non-finite mesh coordinates' ? 'the part position is invalid.'
      : message.startsWith('Cannot place setup diagram label:') ? 'a label could not be placed clearly.'
      : 'the drawing could not be built.';
    return { type: 'paragraph', text: `Setup diagram not drawn: ${reason}` };
  }
}
