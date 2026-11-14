// Renderer-neutral orthographic views. Coordinates remain setup-frame millimetres.
// Visibility uses exact barycentric depth at bounded samples, with a 32² triangle
// index. At most 1M index references, 50k samples and 256 triangles/sample/view.
// Budget exhaustion, degenerate projections and missing geometry stay solid.
export const diagramLegend = [
  ['jaw', 'Vise jaws'], ['parallel', 'Parallels'], ['part', 'Part'],
  ['stock', 'Stock still to face'], ['hidden', 'Hidden below'], ['pickup', 'Pickup feature and zero'],
];
const identity = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
const transform = (p, s) => (s.rotation || identity).map(axis => axis.reduce((v, n, i) => v + n * (p[i] - (s.origin_model?.[i] || 0)), 0));
const pointsOf = (flat) => Array.from({ length: Math.floor(flat.length / 3) }, (_, i) => flat.slice(i * 3, i * 3 + 3));
const bounds = (points) => ({ min: [0, 1, 2].map(i => points.reduce((v, p) => Math.min(v, p[i]), Infinity)), max: [0, 1, 2].map(i => points.reduce((v, p) => Math.max(v, p[i]), -Infinity)) });
const labelFor = (pickup) => {
  if (pickup?.method === 'stock_edges') return 'Stock corner';
  const text = pickup?.description || '';
  const bore = text.match(/⌀[\d.]+ bore/);
  const datum = text.match(/datum [A-Z] face/);
  const face = text.match(/(?:finished )?(?:front|back|left|right|top|bottom) face/);
  return bore?.[0] || datum?.[0] || face?.[0].replace(/^./, c => c.toUpperCase()) || text.split(/[;(]/)[0].replace(/^the /, '').replace(/^./, c => c.toUpperCase());
};

function projection(mesh, setup, vertical) {
  const vertices = pointsOf(mesh?.positions || []).map(p => transform(p, setup));
  const project = p => [p[0], p[vertical], vertical === 1 ? p[2] : -p[1]];
  const pv = vertices.map(project), triangles = [], triangleFaces = [];
  (mesh?.faces || []).forEach(([start, count], face) => { for (let i = start; i < start + count; i++) triangleFaces[i] = face; });
  for (let i = 0; i < (mesh?.indices?.length || 0); i += 3) {
    const t = mesh.indices.slice(i, i + 3).map(j => pv[j]);
    if (t.every(p => p?.every(Number.isFinite))) { t.face = triangleFaces[i / 3]; triangles.push(t); }
  }
  const part = vertices.length ? bounds(vertices) : null;
  const b = pv.length ? bounds(pv) : { min: [0, 0, 0], max: [1, 1, 1] };
  const span = Math.max(b.max[0] - b.min[0], b.max[1] - b.min[1], 1);
  const cell = span / 32, bins = Array.from({ length: 1024 }, () => []);
  const bin = (v, axis) => Math.max(0, Math.min(31, Math.floor((v - b.min[axis]) / cell)));
  let refs = 0, complete = true, samples = 0;
  const area = t => (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1]);
  for (const t of triangles) {
    if (Math.abs(area(t)) < 1e-10) continue;
    const lo = [0, 1].map(a => bin(Math.min(...t.map(p => p[a])), a));
    const hi = [0, 1].map(a => bin(Math.max(...t.map(p => p[a])), a));
    refs += (hi[0] - lo[0] + 1) * (hi[1] - lo[1] + 1);
    if (refs > 1000000) { complete = false; break; }
    for (let y = lo[1]; y <= hi[1]; y++) for (let x = lo[0]; x <= hi[0]; x++) bins[y * 32 + x].push(t);
  }
  function hidden(p) {
    if (!complete || samples >= 50000) return false;
    samples++;
    const candidates = bins[bin(p[1], 1) * 32 + bin(p[0], 0)];
    if (candidates.length > 256) return false;
    for (const t of candidates) {
      const d = area(t);
      const u = ((p[0] - t[0][0]) * (t[2][1] - t[0][1]) - (p[1] - t[0][1]) * (t[2][0] - t[0][0])) / d;
      const v = ((t[1][0] - t[0][0]) * (p[1] - t[0][1]) - (t[1][1] - t[0][1]) * (p[0] - t[0][0])) / d;
      if (u >= -1e-7 && v >= -1e-7 && u + v <= 1 + 1e-7 && t[0][2] + u * (t[1][2] - t[0][2]) + v * (t[2][2] - t[0][2]) > p[2] + span * 1e-5) return true;
    }
    return false;
  }
  // Mesh face ranges provide the silhouette even when no B-rep polylines exist.
  // Weld coincident vertices: meshing duplicates vertices along face boundaries.
  const adjacency = new Map();
  for (const t of triangles) {
    const sign = Math.sign(area(t));
    for (let i = 0; i < 3; i++) {
      const a = t[i], c = t[(i + 1) % 3];
      const key = [a, c].map(p => p.map(v => Math.round(v / (span * 1e-8))).join(',')).sort().join(':');
      const old = adjacency.get(key);
      if (old) { old.signs.add(sign); old.faces.add(t.face); old.count++; }
      else adjacency.set(key, { a, c, signs: new Set([sign]), faces: new Set([t.face]), count: 1 });
    }
  }
  const edges = (mesh?.edges || []).map(e => pointsOf(e).map(p => project(transform(p, setup))));
  for (const e of adjacency.values()) {
    const silhouette = e.count === 1 || (e.signs.has(1) && e.signs.has(-1)) || (e.signs.has(0) && e.signs.size > 1);
    // B-rep wires already contain boundaries between faces. Only add interior
    // curved-face silhouettes there; duplicate strokes would fill dash gaps.
    const interior = e.faces.size === 1 && !e.faces.has(undefined) && e.signs.has(1) && e.signs.has(-1);
    if (silhouette && (!mesh?.edges?.length || interior)) edges.push([e.a, e.c]);
  }
  const lines = [];
  for (const edge of edges) {
    let run = [], state;
    for (let j = 1; j < edge.length; j++) {
      const a = edge[j - 1], c = edge[j];
      const length = Math.hypot(c[0] - a[0], c[1] - a[1]);
      if (length < span * 1e-8) continue;
      const steps = samples >= 50000 ? 1 : Math.min(64, Math.max(1, Math.ceil(length / (span / 160))));
      let last = a.slice(0, 2);
      if (!run.length) run.push(last);
      for (let k = 0; k < steps; k++) {
        const at = f => a.map((v, i) => v + (c[i] - v) * f);
        const h = hidden(at((k + 0.5) / steps));
        if (state !== undefined && h !== state) { lines.push({ type: 'polyline', class: state ? 'hidden' : 'edge', points: run }); run = [last]; }
        state = h; last = at((k + 1) / steps).slice(0, 2); run.push(last);
      }
    }
    if (run.length > 1) lines.push({ type: 'polyline', class: state ? 'hidden' : 'edge', points: run });
  }
  return { part, project, triangles: triangles.filter(t => Math.abs(area(t)) > 1e-10), lines, stats: { triangles: triangles.length, samples, indexComplete: complete } };
}

export function buildSetupDiagram(data, setup, format = String) {
  const c = setup.clamping || {}, stock = setup.stock;
  const located = setup.zero?.status === 'located';
  const warnings = located ? [] : [setup.zero?.status === 'warning' ? `Pickup warning: ${setup.zero.xy?.description || 'X/Y pickup unavailable'}; ${setup.zero.z?.description || 'Z pickup unavailable'}` : 'Pickup warning: no recorded setup zero.'];
  const views = [1, 2].map(vertical => {
    const geometry = projection(data.mesh, setup, vertical), shapes = [];
    const part = geometry.part || stock || { min: [0, 0, -1], max: [1, 1, 0] };
    const size = Math.max(part.max[0] - part.min[0], part.max[vertical] - part.min[vertical], 20), unit = size / 100;
    const add = (type, cls, rest) => shapes.push({ type, class: cls, ...rest });
    const line = (cls, a, b) => add('polyline', cls, { points: [a, b] });
    const text = (cls, x, y, text, anchor = 'start') => add('text', cls, { x, y, text, anchor });
    const annotations = [];
    const annotation = (from, x, y, caption, anchor = 'start') => {
      const end = [x, y - 2 * unit];
      line('leader', from, end); text('pickup', x, y, caption, anchor);
      annotations.push({ shape: shapes.at(-1), end });
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
    for (const jaw of jaws) rect('jaw', jaw);
    for (const p of c.parallels || []) rect('parallel', p);
    // Fill only projected triangles, preserving holes, concavities and overhangs.
    for (const t of geometry.triangles) add('polygon', 'part', { points: t.map(p => p.slice(0, 2)) });
    shapes.push(...geometry.lines);
    if (!geometry.part) text('tag', part.min[0], part.max[vertical] + 5 * unit, 'Part mesh unavailable');
    // Only the material above the finished envelope is a facing allowance.
    if (stock) {
      if (vertical === 1) rect('stock', stock);
      else if (stock.max[2] > part.max[2] + 1e-4) rect('stock', { min: [stock.min[0], 0, part.max[2]], max: stock.max });
    }
    const jawAxis = c.fixed_jaw?.startsWith('x') ? 0 : 1;
    const sorted = (c.jaws || []).map((j, i) => ({ j, i })).sort((a, b) => a.j.min[jawAxis] - b.j.min[jawAxis]);
    const fixedIndex = c.fixed_jaw ? sorted[c.fixed_jaw.endsWith('min') ? 0 : sorted.length - 1]?.i : -1;
    jaws.forEach((j, i) => text('tag', (j.min[0] + j.max[0]) / 2, vertical === 1 ? j.max[1] - 4 * unit : j.min[2] + (jawAxis === 1 && i !== fixedIndex ? 10 : 5) * unit, i === fixedIndex ? 'Fixed jaw' : fixedIndex < 0 ? 'Jaw (fixed not recorded)' : 'Moving jaw', 'middle'));
    function dimension(x, low, high, caption, value, anchor = 'start') {
      if (!Number.isFinite(value) || high <= low) return;
      line('dimension', [x, low], [x, high]);
      for (const y of [low, high]) line('dimension', [x - 1.6 * unit, y], [x + 1.6 * unit, y]);
      text('dimension', x + (anchor === 'end' ? -2.5 : 2.5) * unit, (low + high) / 2, `${caption} ${format(value)}`, anchor);
    }
    if (vertical === 2) {
      const p = c.parallels?.[0];
      if (p) dimension(part.min[0] + 18 * unit, p.min[2], p.max[2], 'Parallels', p.max[2] - p.min[2]);
      if (c.grip_height_mm != null) dimension(part.min[0] - 6 * unit, c.jaw_top_z - c.grip_height_mm, c.jaw_top_z, 'Grip', c.grip_height_mm, 'end');
      if (stock) dimension(part.max[0] + 8 * unit, part.max[2], stock.max[2], 'Facing', stock.max[2] - part.max[2]);
    }
    if (located) {
      const r = 2.6 * unit;
      line('axis', [0, 0], [14 * unit, 0]); line('axis', [0, 0], [0, 14 * unit]);
      add('circle', 'zero', { x: 0, y: 0, radius: r });
      for (const start of [0, Math.PI]) add('polygon', 'zero-fill', { points: [[0, 0], ...Array.from({ length: 9 }, (_, i) => [r * Math.cos(start + i * Math.PI / 16), r * Math.sin(start + i * Math.PI / 16)])] });
      text('pickup', 15 * unit, 0, 'X'); text('pickup', 0, 16 * unit, vertical === 1 ? 'Y' : 'Z');
      text('pickup', 6 * unit, -6 * unit, vertical === 1 ? (setup.zero.xy?.axes ? 'X0 Y0' : labelFor(setup.zero.xy)) : 'Z0 here');
      const pickups = [setup.zero.xy, ...Object.values(setup.zero.xy?.axes || {}), setup.zero.z];
      const ids = new Set(pickups.flatMap(p => p?.target_features?.length ? p.target_features : p?.features || []));

      // Highlight the actual mesh boundary of each picked face, not its box.
      for (const id of ids) {
        const faceIds = id.startsWith('faces:') ? id.slice(6).split('.').map(Number) : data.analysis?.features?.find(f => f.id === id)?.faces || [];
        for (const face of faceIds) {
          const range = data.mesh?.faces?.[face];
          if (!range) continue;
          const boundary = new Map();
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
          const record = data.analysis?.faces?.find(f => f.id === face);
          if (record?.centroid) {
            const p = geometry.project(transform(record.centroid, setup));
            const pickup = [...Object.values(setup.zero.xy?.axes || {}), setup.zero.z, setup.zero.xy].find(p => p?.features?.includes(id) || p?.target_features?.includes(id));
            const caption = labelFor(pickup) || `Face ${face}`;
            const normal = (setup.rotation || identity).map(axis => axis.reduce((v, n, i) => v + n * (record.direction?.[i] || 0), 0));
            const nx = normal[0], ny = normal[vertical];
            let tx = p[0], ty = part.max[vertical] + 12 * unit, anchor = 'middle';
            let from = [p[0], part.max[vertical]];
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
        const contact = setup.zero.xy?.axes?.[stop.axis]?.contact_in_previous_frame_mm;
        const p = contact ? contact.map((v, i) => v - (setup.zero.offset_in_previous_frame_mm?.[i] || 0)) : [0, 0, stop.contact_z_mm];
        p[stop.axis === 'x' ? 0 : 1] = stop.coordinate_mm; p[2] = stop.contact_z_mm;
        const q = geometry.project(p), top = stop.maximum_z_mm ?? stop.contact_z_mm;
        const axis = stop.axis === 'x' ? 0 : 1;
        const negativeSide = stop.coordinate_mm <= (part.min[axis] + part.max[axis]) / 2;
        const along = negativeSide ? 6 : 0, height = vertical === 2 ? Math.max(6 * unit, top - stop.contact_z_mm) : 6 * unit;
        add('rect', 'stop', { x: q[0] - (axis === 0 ? along : 3) * unit, y: vertical === 1 ? q[1] - (axis === 1 ? along : 3) * unit : top - height, width: 6 * unit, height, contact: p, maximum_z_mm: stop.maximum_z_mm });
        annotation(q.slice(0, 2), q[0] + 5 * unit, vertical === 1 ? q[1] - 12 * unit : top + 9 * unit, `Stop${stop.maximum_z_mm == null ? '' : ` · nose ≤ Z${format(stop.maximum_z_mm)}`}`);
      }
    }
    // A conservative font-width estimate keeps pickup labels apart in both
    // renderers. Search a fixed number of rows; this never depends on the DOM.
    const textBox = s => {
      const width = s.text.length * 3.4 * 0.62 * unit;
      const x = s.x - (s.anchor === 'middle' ? width / 2 : s.anchor === 'end' ? width : 0);
      return [x, s.y - unit, x + width, s.y + 3.8 * unit];
    };
    const placed = shapes.filter(s => s.type === 'text' && !annotations.some(a => a.shape === s)).map(textBox);
    for (const { shape, end } of annotations) {
      const initialY = shape.y;
      for (let row = 0; row < 24; row++) {
        shape.y = initialY + row * 6 * unit;
        const b = textBox(shape);
        if (placed.every(a => b[2] + unit < a[0] || b[0] - unit > a[2] || b[3] + unit < a[1] || b[1] - unit > a[3])) break;
      }
      end[1] = shape.y - 2 * unit; placed.push(textBox(shape));
    }
    const extents = shapes.flatMap(s => s.points || (s.type === 'rect' ? [[s.x, s.y], [s.x + s.width, s.y + s.height]] : [[s.x, s.y]]));
    // Reserve label space on either side, without changing geometry dimensions.
    const b = bounds(extents.map(p => [...p, 0]));
    return { caption: vertical === 1 ? 'Seen from above (machine Z down)' : 'Seen from the front (looking +Y)', shapes, unit, bounds: [b.min[0] - 28 * unit, b.min[1] - 9 * unit, b.max[0] + 42 * unit, b.max[1] + 9 * unit], cropped, stats: geometry.stats };
  });
  return { type: 'setup-diagram', setup: setup.id, views, warnings, legend: diagramLegend, note: views[0].cropped ? 'Jaws cropped in Y in the view from above.' : '' };
}
