// Shared page wording and renderer-neutral report content. No DOM or PDF dependency.

// specify-core's descriptions of the part as a whole, not features a person would point at.
export const NOT_FEATURES = new Set(["hole_patterns", "step_levels", "risers", "section_recesses", "plates", "thin_wall_bodies", "sheet_metal_bodies", "turned_bodies", "circular_face_patterns"]);

export const trim = (x) => Number.isNaN(Number(x)) ? "—" : String(Math.round(Number(x) * 10) / 10);

export function featureName(f, format = trim) {
  const r = f.record || {};
  switch (f.family) {
    case "holes": return `⌀${format(r.diameter)} hole`;
    case "slots": return `Slot, ${format(r.width)} wide`;
    case "pockets": return r.open_sign ? `Notch, ${format(r.width)} wide × ${format(r.depth)} deep` : `Pocket, ${format(r.width)} wide × ${format(r.depth)} deep`;
    case "v_channels": return `V-channel, ${format(r.width)} wide × ${format(r.depth)} deep`;
    case "polygonal_bosses": return r.side_count === 6 ? `Hex boss, ${format(r.across_flats)} across flats` : `${r.side_count ?? "—"}-sided boss`;
    case "fillets": return r.side === "convex" ? `r${format(r.radius)} edge round` : `r${format(r.radius)} fillet`;
    default: return (f.family || "—").replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
  }
}

export function operationName(op, setup, plan, format = null) {
  const size = format || ((x) => x ?? "—"), rounded = format || trim;
  const where = plan.setups.length > 1 ? ` (${setup.id})` : "";
  if (op.kind === "spot_drill") return "Spot drill" + where;
  if (op.kind === "facing") return "Facing" + where;
  if (op.kind === "roughing") return "Roughing" + where;
  if (op.kind === "counterbore") return `⌀${size(op.diameter)} counterbore` + where;
  if (op.kind === "thread") return `${op.spec ?? "—"} thread` + where;
  if (op.kind === "slot") return `${size(op.width)} × ${size(op.length)} slot` + where;
  if (op.kind === "notch") return `${rounded(op.width)} × ${rounded(op.length)} ${op.open ? "notch" : "slot"}` + where;
  if (op.kind === "v_channel") return `V-channel, ${rounded(op.width)} wide` + where;
  if (op.kind === "flats") return "Finish flats" + where;
  if (op.kind === "wall_finish") return "Finish outside wall" + where;
  if (op.kind === "edge_chamfer") return (op.radius == null ? `${size(op.width)} mm 45° edge chamfer` : `${size(op.width)} chamfer on r${size(op.radius)} edges`) + where;
  if (op.kind === "holes") return `⌀${size(op.diameter)} ${op.bottom === "through" ? "through" : op.bottom === "drill_point" ? "drill point" : op.bottom}` + where;
  return `⌀${size(op.diameter)} ${op.bottom === "through" ? "through" : op.bottom === "drill_point" ? "drill point" : op.bottom}` + where;
}

export const describeTime = (s) => `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;

export const escapeHtml = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export const STAGES = { machine: "Machine", requirements: "Requirement", setup: "Setup", process: "Process", sequence: "Order", feasibility: "Feasibility", tools: "Tool", execution: "Execution" };

export function describeSource(src) {
  const what = src.kind === "standard" ? src.ref : src.kind === "practice" ? "recognised practice" : "step2cam's own judgement";
  const via = src.via ? ` (${src.via})` : "";
  const note = src.note ? `: ${src.note}` : "";
  return `${what}${note}${via}`;
}

export function planWarnings(m) {
  return (m.data.plan.decisions || []).filter((d) => d.outcome === "warning" && !d.superseded_by);
}

export function warningOperations(m, d) {
  return [...m.operations.values()].filter((o) => o.because?.includes(d.id));
}

export function warningText(m, d, brief = false) {
  let text = d.says.replace("no source-admitted entry/route for component 0; choose a smaller radius or another entry strategy", "camlib found no way for that tool to enter it");
  if (brief && d.depth_diameter_ratio != null) text = `${Number(d.depth_diameter_ratio).toFixed(1)} × diameter deep: full-retract pecks`;
  else if (brief && d.rule.startsWith("machine.clamping@") && d.jaw_cap_z != null) text = d.subject.endsWith(":roughing")
    ? `side cutting stops at Z${trim(d.jaw_cap_z)} (jaws)`
    : `below Z${trim(d.jaw_cap_z)} jaw clearance: left for a later setup`;
  else if (brief && d.wall_stock_mm != null) text = `Walls left up to ${Number(d.wall_stock_mm).toFixed(3)} mm thick at the top`;
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

export function checkText(programs) {
  const setupName = (id) => `${id[0].toUpperCase()}${id.slice(1)} setup`;
  const checked = programs.filter((p) => p.material), unchecked = programs.filter((p) => !p.material);
  const headline = [];
  if (checked.length) headline.push(`<span class="ok">✓ ${checked.map((p) => escapeHtml(setupName(p.setup))).join(", ")}: compared with the part, no gouge found.</span>`);
  if (unchecked.length) headline.push(`<span class="unchecked">${unchecked.map((p) => escapeHtml(setupName(p.setup))).join(", ")}: not compared with the part.</span>`);

  const compact = checked.length
    ? `✓ no gouge${unchecked.length ? ` · ${unchecked.length} unchecked` : ""}`
    : "⚠ unchecked";
  const number = (x) => Math.round(x).toLocaleString("en-GB");
  const items = [];
  for (const p of checked) {
    const { limits, gouge, residual } = p.material;
    const where = checked.length > 1 ? `${setupName(p.setup)}: ` : "";
    const say = (text) => items.push(where ? where + text : text[0].toUpperCase() + text.slice(1));
    const cell = limits.sideways_mm;
    say(`too small for the check to see: errors under about ${limits.height_mm.toFixed(2)} mm in height or ${cell.toFixed(2)} mm sideways, and misplaced cuts narrower than about ${(4 * cell).toFixed(1)} mm.`);
    say(`it can't tell for ${number(gouge.volume_mm3[1])} mm³ beside walls and curved faces, because it sees the part from above.`);
    say(`at least ${number(residual.volume_mm3[0])} mm³ of stock is still there. That includes stock outside the part, and anything a later setup cuts.`);
    if (limits.before_threads) say(`threads are not compared, because they go beyond the bore the model shows.`);
  }
  if (unchecked.length) items.push("Not compared: only the setup with roughing holds the part's model in its position, so side holes and the like rely on the other checks.");
  return { headline: headline.join(" "), compact, items };
}

export function workholdingText(c) {
  const seconds = Math.round(c.estimated_seconds || 0);
  const duration = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  const leftOut = c.features_left_out?.length || 0;
  const needs = [...new Set((c.tools_needed || []).map((n) => n.description.split(/,? or /)
    .map((option) => option.split(/\s+(?:for|with|whose|under|of|covering)\b/)[0]).join(" or ")))];
  const softJaws = c.grips?.some((g) => g.finished_wall) ? " · soft jaws advised" : "";
  const summary = c.available
    ? `${c.finished ? "finished" : c.reason ?? "—"}${c.finished && leftOut ? ` · ${leftOut} features left out` : ""} · ${c.setups ?? "—"} setup${c.setups === 1 ? "" : "s"} · +${c.extra_stock_mm == null ? "—" : Number(c.extra_stock_mm.toFixed(3))} mm stock · ${duration}${softJaws}${needs.length ? ` · needs ${needs.join("; ")}` : ""}`
    : c.short_reason || c.reason;
  const label = c.id === "fixture_plate" ? "Fixture plate" : c.label;
  return { label, summary };
}

export function setupZero(setup, format = trim) {
  const lo = setup.stock?.min, hi = setup.stock?.max;
  if (setup.zero?.xy) return `Tool comes down −Z; X0 Y0: ${setup.zero.xy.description} · Z0: ${setup.zero.z.description}; stock ${lo && hi ? hi.map((v, i) => format(v - lo[i])).join(" × ") : "—"} mm.`;
  return `Tool comes down −Z; zero on the ${setup.zero === "stock" ? "stock" : "part's box"}, stock ${lo && hi ? hi.map((v, i) => format(v - lo[i])).join(" × ") : "—"} mm.`;
}

const value = (x) => x == null || x === "" ? "—" : typeof x === "object" ? JSON.stringify(x) : String(x);
const text = value;
export const reportLength = (x) => {
  if (x == null) return "—";
  const n = Number(x);
  if (!Number.isFinite(n)) return "—";
  if (Math.abs(n - 3.175) < 1e-10) return "3.175"; // coordinate subtraction can lose a few ulps
  if (n !== 0 && Math.abs(n) < 0.1) return String(Number(n.toPrecision(3)));
  return String(Math.sign(n) * Math.round((Math.abs(n) + Number.EPSILON * Math.max(1, Math.abs(n))) * 100) / 100);
};
export function operationTime(seconds) {
  const s = Math.round(seconds);
  const minutes = Math.round(s / 60);
  return s >= 3600 ? `${Math.floor(minutes / 60)} h ${minutes % 60} min`
    : `${Math.floor(s / 60)} min ${s % 60} s`;
}
function jsonSummary(x) {
  if (Array.isArray(x)) return x.map(jsonSummary).join(", ");
  if (x && typeof x === "object") {
    // The grip-band trace records implicated operations; retain their exact ids.
    if (typeof x.operation === "string") {
      const more = Object.entries(x).filter(([key]) => key !== "operation").map(([key, v]) => `${key.replaceAll("_", " ")}: ${key.endsWith("_mm") && typeof v === "number" ? reportLength(v) : jsonSummary(v)}`);
      return x.operation + (more.length ? ` (${more.join("; ")})` : "");
    }
    return Object.entries(x).map(([key, v]) => `${key.replaceAll("_", " ")}: ${key.endsWith("_mm") && typeof v === "number" ? reportLength(v) : jsonSummary(v)}`).join("; ");
  }
  return String(x);
}
const rate = (x) => x == null ? "—" : String(Math.round(x));
export function reportDuration(seconds) {
  const minutes = Math.round(seconds / 60);
  return `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")} min`;
}
// Keep the page's established wording; only the machinist's sheet simplifies it.
export function sheetWords(input) {
  let s = value(input);
  const summaries = [];
  // Balanced JSON objects/arrays can contain nested values and escaped quotes.
  for (let start = 0; start < s.length; start++) {
    if (!"{[".includes(s[start])) continue;
    let depth = 0, quoted = false, escaped = false;
    for (let end = start; end < s.length; end++) {
      const c = s[end];
      if (quoted) { if (escaped) escaped = false; else if (c === "\\") escaped = true; else if (c === '"') quoted = false; }
      else if (c === '"') quoted = true;
      else if ("{[".includes(c)) depth++;
      else if ("}]".includes(c) && --depth === 0) {
        const part = s.slice(start, end + 1);
        try { const summary = jsonSummary(JSON.parse(part)); const marker = `§JSON${summaries.length}§`; summaries.push(summary); s = s.slice(0, start) + marker + s.slice(end + 1); start += marker.length - 1; } catch { /* ordinary bracketed prose */ }
        break;
      }
    }
  }
  const names = {
    "Closed_slot with Radiused_slot_end_type ends": "slot with rounded ends",
    "bottom_and_side_finish_milling": "finish bottom and sides",
    "Vee_profile": "V-channel", "full_retract": "full-retract pecks",
    "Closed_slot": "closed slot", "Radiused_slot_end_type": "rounded ends",
  };
  for (const [id, words] of Object.entries(names)) s = s.replaceAll(id, words);
  s = s.replace(/\b[A-Za-z][A-Za-z0-9]*(?:_[A-Za-z0-9]+)+\b/g, (id) => id.replaceAll("_", " "))
    // Prose is not a bag of lengths: preserve pitches, ratios, ids and other numbers.
    .replace(/(-?\d+\.\d+)(?=\s*mm\b)/g, (n) => reportLength(n))
    .replace(/(⌀|\br|\b[XYZ]\s*)(-?\d+\.\d+)/g, (_, prefix, n) => prefix + reportLength(n));
  return s.replace(/§JSON(\d+)§/g, (_, index) => summaries[Number(index)]).replace(/ {2,}/g, " ").trim();
}
function threadSpecification(analysis, feature) {
  const faces = feature?.faces || [];
  if (!faces.length) return null;
  for (const [key, answer] of Object.entries(analysis?.resume?.answers || {})) {
    if (!key.startsWith("hole.function:") || typeof answer !== "string" || !answer.startsWith("tapped:")) continue;
    const supported = key.split(":").at(-1).split(".").map(Number);
    if (faces.every((face) => supported.includes(face))) return answer.slice(7).replace(/x/gi, "×");
  }
  return null;
}
function setupBox(part, setup) {
  if (!part?.min || !part?.max || !setup.rotation || !setup.origin_model) return null;
  const points = Array.from({ length: 8 }, (_, bits) => setup.rotation.map((axis) => axis.reduce((sum, n, i) => sum + n * ((bits & (1 << i) ? part.max[i] : part.min[i]) - setup.origin_model[i]), 0)));
  return { min: [0, 1, 2].map((i) => Math.min(...points.map((p) => p[i]))), max: [0, 1, 2].map((i) => Math.max(...points.map((p) => p[i]))) };
}
export function setupDetails(setup, plan) {
  const unstated = "not stated in the plan", c = setup.clamping || {};
  const face = (id) => typeof id !== "string" ? unstated : id.replace(/^(stock|part)\.([xyz])\.(min|max)$/, (_, body, axis, edge) => `${body} ${edge === "min" ? "−" : "+"}${axis.toUpperCase()} face`);
  const lines = [setup.before_setup?.says, setup.stock_note].filter(Boolean);
  lines.push(`Faces in jaws (setup axes): ${c.faces?.length ? c.faces.map(face).join(", ") : unstated}. Jaw-top Z: ${c.jaw_top_z == null ? unstated : reportLength(c.jaw_top_z) + " mm"}. Grip depth: ${c.grip_height_mm == null ? unstated : reportLength(c.grip_height_mm) + " mm"}.`);
  if (c.contacts?.length) lines.push(`Contact faces (model axes): ${[...new Set(c.contacts.map((contact) => face(contact.face_model)))].join(", ")}.`);
  lines.push(c.parallels?.length ? c.parallels.map((p, i) => {
    if (!p.min || !p.max) return `Parallel ${i + 1}: height/width ${unstated}`;
    const spans = p.max.map((v, n) => v - p.min[n]);
    return `Parallel ${i + 1}: height ${reportLength(spans[2])} mm${c.parallels_size?.kind === "standard" ? " — standard size, not checked against your shop" : c.parallels_size?.kind === "computed" ? " — not a standard size" : ""}, width ${reportLength(Math.min(spans[0], spans[1]))} mm (from support-box dimensions)`;
  }).join("; ") + "." : `Parallels: none modelled; physical support height/width ${unstated}.${c.parallels_size?.kind === "computed" ? " Not a standard size." : ""}`);
  const name = setup.zero === "stock" ? "stock" : setup.zero === "part" ? "part box" : null;
  const box = setup.zero === "stock" ? setup.stock : setup.zero === "part" ? setupBox(plan.part, setup) : null;
  const zero = (x) => Math.abs(x) < 1e-7;
  let xy = unstated, z = unstated;
  if (box?.min && box?.max && name) {
    const edges = [0, 1].map((i) => zero(box.min[i]) ? "minimum" : zero(box.max[i]) ? "maximum" : null);
    if (edges.every(Boolean)) xy = `${edges[0]}-X/${edges[1]}-Y corner of the ${name} (setup axes)`;
    else if ([0, 1].every((i) => zero((box.min[i] + box.max[i]) / 2))) xy = `centre of the ${name}`;
    if (zero(box.max[2])) z = `top of the ${name} in setup coordinates`;
    else if (zero(box.min[2])) z = `bottom of the ${name} in setup coordinates`;
    else z = `${reportLength(box.max[2])} mm below the top of the ${name}`;
  }
  lines.push(`Zero   X0 Y0: ${setup.zero?.xy?.description || xy}\n       Z0: ${setup.zero?.z?.description || z}`);
  return lines;
}
const heading = (text, level = 2) => ({ type: "heading", level, text });
const paragraph = (text) => ({ type: "paragraph", text });
const list = (items) => ({ type: "list", items: items.length ? items : ["—"] });
const table = (headers, rows, id = "") => ({ type: "table", id, headers, rows: rows.length ? rows : [headers.map(() => "—")] });
const row = (cells) => cells;
const pageBreak = () => ({ type: "page-break" });

/** A renderer-neutral document, with no page DOM or browser dependencies.
 * Maps may supply the page's move-derived operation times and feature annotations.
 * Without them, times are calculated from the same program moves and feed rates.
 */
export function reportDocument(data, { rules = new Map(), library = {}, operations, features, estimatedSeconds, date = new Date().toLocaleDateString("en-GB") } = {}) {
  const plan = data.plan;
  const tools = data.tools || library.tools || [];
  const machine = library.machines?.find((m) => m.id === data.library?.machine);
  const programs = data.error ? [] : data.programs || [];
  const times = new Map();
  let total = 0;
  for (const p of programs) for (const move of p.moves || []) {
    const rate = move.kind === "rapid" ? data.rapid_mm_min || 5000 : move.feed || data.rapid_mm_min || 5000;
    let seconds = 0;
    for (let i = 1; i < (move.points || []).length; i++) seconds += Math.hypot(...move.points[i].map((v, k) => v - move.points[i - 1][k])) / rate * 60;
    const id = move.op?.includes("-spot_drill-") ? move.op.split("-spot_drill-")[0] + "-spot_drill" : move.op;
    times.set(id, (times.get(id) || 0) + seconds);
    total += seconds;
  }
  const fs = new Map([...(features?.values() || (data.analysis?.features || []).filter((f) => !NOT_FEATURES.has(f.family)))].map((f) => [f.id, { ...f, name: featureName(f, reportLength) }]));
  const originalOps = operations || new Map(plan.setups.flatMap((s) => s.operations.map((o, i) => {
    const id = `${s.id}-${i + 1}-${o.kind}`;
    return [id, { ...o, id, setup: s, name: operationName(o, s, plan), time: times.get(id) || 0 }];
  })));
  const ops = new Map([...originalOps].map(([id, o]) => [id, { ...o, name: operationName(o, o.setup, plan, reportLength) }]));
  const m = { data, operations: ops, features: fs };
  const warnings = planWarnings(m);
  const check = checkText(programs);
  const colletWarnings = warnings.filter((d) => d.rule.startsWith("machine.spindle.manual_collets@"));
  const manualCollets = machine?.spindle?.manual_collets || machine?.spindle?.collet_change === "manual" || colletWarnings.length > 0;
  const chosen = plan.workholding?.candidates?.find((c) => c.id === plan.workholding.chosen);
  const needs = plan.tools_needed || [];
  const decisions = new Map((plan.decisions || []).map((d) => [d.id, d]));
  const used = [...new Set(plan.setups.flatMap((s) => s.operations.map((o) => o.tool)).filter(Boolean))];
  const toolLabel = (id) => {
    const t = tools.find((t) => t.id === id);
    return `${t?.number == null ? "T—" : `T${text(t.number)}`} · ${text(id)}`;
  };
  const missing = needs.map((n) => ({ ...n, context: "Current plan" }));
  const missingByTool = new Map();
  for (const n of missing) {
    if (!missingByTool.has(n.description)) missingByTool.set(n.description, { contexts: new Set(), features: new Set() });
    const entry = missingByTool.get(n.description);
    entry.contexts.add(n.context);
    for (const id of n.features || []) entry.features.add(id);
  }
  const blocks = [];
  const stockSize = (stock) => stock ? stock.max.map((v, i) => reportLength(v - stock.min[i])).join(" × ") : "—";
  blocks.push(heading("SETUP SHEET", 1), heading(text(data.name)), paragraph("Draft — check before cutting"));
  blocks.push(table(["Machine", "Date", "Material", "Stock size (mm)", "Feed-rate estimate"], [row([
    text(machine?.name || data.library?.machine).replace(/^the\s+/i, ""), text(date), text(plan.material?.used), stockSize(plan.setups[0]?.stock) + (plan.setups[0]?.stock_size?.kind === "standard" ? " — standard size, not checked against your shop" : plan.setups[0]?.stock_size?.kind === "computed" ? " — not a standard size" : ""),
    programs.length ? reportDuration(estimatedSeconds ?? total) : "—",
  ])], "header"));
  blocks.push(paragraph(sheetWords(`Camlib: ${data.error ? `could not run the plan: ${data.error}` : programs.length ? check.compact : "—"}`)));
  if (data.notice) blocks.push(paragraph(sheetWords(data.notice)));
  blocks.push(heading("TOOLS — load before starting"));
  blocks.push(table(["T / tool id", "Type / ⌀ mm", "Flute length mm", "Stickout / overall mm", "RPM", "Feed mm/min"], used.map((id) => {
    const t = tools.find((t) => t.id === id);
    const uses = [...ops.values()].filter((o) => o.tool === id);
    const settings = (key) => [...new Set(uses.map((o) => rate(o.technology?.[key])))].join(" / ");
    return row([toolLabel(id), `${sheetWords(t?.type)} / ${reportLength(t?.diameter)}`, reportLength(t?.flute_length), `${reportLength(t?.stickout)} / ${reportLength(t?.overall_length)}`, settings("spindle_rpm"), settings("feed_mm_min")]);
  }), "tools"));
  if (manualCollets) {
    blocks.push(paragraph("Manual collets: check the collet at every tool change."));
    blocks.push(paragraph("Collet changes — " + plan.setups.map((setup) => {
      const recorded = colletWarnings.find((d) => d.subject === setup.id)?.shank_sizes_mm;
      const shanks = recorded ? recorded.map(reportLength) : [...new Set(setup.operations.filter((o) => o.tool).map((o) => reportLength(tools.find((t) => t.id === o.tool)?.shank_diameter)))];
      return `${setup.id}: ${shanks.join(", ") || "—"} mm shanks`;
    }).join("; ")));
    if (colletWarnings.length) blocks.push(list(colletWarnings.map((d) => sheetWords(warningText(m, d)))));
  }
  if (missingByTool.size) blocks.push(heading("Tools needed but not in the library"), list([...missingByTool].map(([description, n]) => sheetWords(`${text(description)} — needed by ${[...n.contexts].map(text).join("; ")}${n.features.size ? ` — ${[...n.features].map((id) => text(fs.get(id)?.name || id)).join(", ")}` : ""}`))));
  let previousTool;
  for (const setup of plan.setups) {
    const grip = chosen?.grips?.find((g) => g.setup === setup.id);
    const holding = chosen ? workholdingText(chosen).label : null;
    const details = setupDetails(setup, plan), zeroLines = details.pop().split("\n");
    blocks.push(pageBreak(), heading(`SETUP — ${setup.id}`, 1),
      paragraph(sheetWords(`Workholding: ${text(holding)}${setup.flip_seating ? " · flip onto parallels" : ""}${grip?.finished_wall ? " · soft jaws advised" : ""}`)),
      ...details.map(paragraph),
      table([], [["Zero", zeroLines[0].replace(/^Zero\s+/, "")], ["", zeroLines[1].trimStart()]], "setup-zero"));
    const rows = setup.operations.map((op, i) => {
      const id = `${setup.id}-${i + 1}-${op.kind}`, o = ops.get(id), t = op.technology || {};
      const notes = [];
      if (t.peck_mode != null) notes.push(`Peck mode: ${t.peck_mode}`);
      if (t.peck_mm != null) notes.push(`Peck depth: ${reportLength(t.peck_mm)} mm`);
      if (op.tool_note) notes.push(op.tool_note);
      if (op.notes) notes.push(value(op.notes));
      // Keep geometric shape information; ISO operation-taxonomy rationale stays
      // in the appendix alongside the original note.
      if (op.kind === "slot" && op.iso14649?.note?.includes("Radiused_slot_end_type")) notes.push("slot with rounded ends");
      if (op.kind === "v_channel" && op.iso14649?.note?.includes("Vee_profile")) notes.push("V-channel cut by one V-bit pass along it");
      const old = tools.find((t) => t.id === previousTool), next = tools.find((t) => t.id === op.tool);
      if (manualCollets && previousTool && previousTool !== op.tool) {
        if (old?.shank_diameter != null && next?.shank_diameter != null) {
          if (old.shank_diameter !== next.shank_diameter) notes.push(`Manual collet change: ${previousTool} → ${op.tool} (${reportLength(old.shank_diameter)} → ${reportLength(next.shank_diameter)} mm shank).`);
        } else notes.push(`Check manual collet change: ${previousTool} → ${op.tool}; shank size —.`);
      }
      if (op.tool) previousTool = op.tool;
      notes.push(...warnings.filter((d) => op.because?.includes(d.id) && !d.rule.startsWith("machine.spindle.manual_collets@") && !d.rule.startsWith("execution.refused@"))
        .map((d) => t.peck_mode && d.depth_diameter_ratio != null ? warningText(m, d, true).replace(/: full-retract pecks$/, "") : warningText(m, d, true)));
      // Notes stay in the operation's row so the PDF never separates them.
      return row([text(i + 1), sheetWords(o?.name || operationName(op, setup, plan)) + (notes.length ? "\n" + notes.map(sheetWords).join("\n") : ""), toolLabel(op.tool).replace(" · ", "\n"), rate(t.spindle_rpm), rate(t.feed_mm_min), rate(t.plunge_mm_min), `${reportLength(t.stepdown_mm)} / ${reportLength(t.stepover_mm)}`, o?.time ? operationTime(o.time) : "not run"]);
    });
    blocks.push(table(["#", "Operation / notes", "Tool", "RPM", "Feed mm/min", "Plunge mm/min", "Stepdown / stepover mm", "Time est."], rows, "operations"));
  }
  const refused = (plan.decisions || []).filter((d) => d.rule?.startsWith("execution.refused@") && d.outcome === "removed");
  const skippedRows = (plan.skipped || []).map((skip) => {
    const decision = decisions.get(skip.decision), f = fs.get(skip.feature);
    let label = f?.name || skip.feature, reason = skip.reason, affectedOperation = false;
    if (decision?.rule.startsWith("execution.refused@")) {
      const removal = refused.find((r) => (plan.decisions || []).some((d) => d.features?.includes(skip.feature)
        && (d.superseded_by === r.id || d.rule.startsWith("execution.refused@") && d.subject === r.subject)));
      if (removal?.message) reason = `left out: ${removal.message}`;
      const id = removal?.subject, op = removal?.operation, kind = op && (op.part == null ? op.kind : `${op.kind}-${op.part}`);
      if (kind === "thread") {
        const spec = threadSpecification(data.analysis, f);
        const drilled = [...ops.values()].some((o) => o.kind === "holes" && o.time > 0 && o.features?.includes(skip.feature));
        label = `${spec ? spec + " " : ""}thread (${id}) — not cut${drilled ? " (hole is drilled)" : ""}`;
      } else if (id) { label = `${kind?.replaceAll("_", " ") || "Operation"} (${id}) — not cut`; affectedOperation = true; }
      else if ([...ops.values()].some((o) => o.features?.includes(skip.feature))) label = `Refused machining of ${label} — not cut`;
    }
    return { cells: [sheetWords(label), sheetWords(reason), sheetWords(needs.filter((n) => n.features?.includes(skip.feature)).map((n) => n.description).join("; "))], affectedOperation };
  }).concat([...fs.values()].filter((f) => f.notCut && !plan.skipped?.some((skip) => skip.feature === f.id)).map((f) => ({ cells: [sheetWords(f.name), sheetWords(f.note), "—"] })));
  const groupedSkips = new Map();
  for (const { cells, affectedOperation } of skippedRows) {
    const key = JSON.stringify(cells);
    if (!groupedSkips.has(key)) groupedSkips.set(key, { cells, count: 0, affectedOperation });
    groupedSkips.get(key).count++;
  }
  blocks.push(heading("NOT CUT"), table(["Feature / operation", "Reason", "Tool that would cut it"], [...groupedSkips.values()].map(({ cells, count, affectedOperation }) => [count > 1 ? affectedOperation ? `${cells[0]} (${count} affected features)` : `${count} × ${cells[0]}` : cells[0], ...cells.slice(1)]), "not-cut"));
  blocks.push(heading("WARNINGS"));
  const groups = new Map();
  const sheetWarnings = warnings.filter((d) => !d.rule.startsWith("machine.spindle.manual_collets@") && !d.rule.startsWith("execution.refused@"));
  for (const d of sheetWarnings) {
    const label = rules.get(d.rule)?.warning_label || "plan needs checking";
    if (!groups.has(label)) groups.set(label, []);
    groups.get(label).push(sheetWords(warningText(m, d)));
  }
  for (const [label, items] of groups) blocks.push(heading(sheetWords(label), 3), list(items));
  const extra = (plan.warnings || []).filter((w) => !warnings.some((d) => d.says === w));
  if (extra.length) blocks.push(list(extra.map(sheetWords)));
  if (!sheetWarnings.length && !extra.length) blocks.push(paragraph("—"));
  if (refused.length) blocks.push(heading("Refused by camlib", 3), list(refused.map((d) => sheetWords(`${d.subject}: ${d.says}`))));
  const appendixStart = blocks.length;
  blocks.push(pageBreak(), heading("APPENDIX — FULL REPORT", 1), heading("Recognised features"), table(["Id", "Feature", "What happens to it"], [...fs.values()].map((f) => {
    const cutBy = [...ops.values()].filter((o) => o.features?.includes(f.id));
    const disposition = f.note || (cutBy.length ? [...new Set(cutBy.map((o) => o.name))].join("; ")
      : (plan.skipped || []).some((s) => s.feature === f.id) || f.notCut ? "not cut"
      : (plan.covered_by_roughing || []).includes(f.id) ? "covered by roughing" : "not cut");
    return row([text(f.id), sheetWords(f.name || featureName(f)), text(disposition)]);
  }), "features"));
  blocks.push(heading("Workholding options and consequences"));
  for (const c of plan.workholding?.candidates || []) {
    const { label, summary } = workholdingText(c);
    blocks.push(paragraph(`${c.id === plan.workholding.chosen ? "✓" : c.available ? "○" : "✕"} ${label} — ${summary}. ${text(c.comparison)}`),
      list([...(c.risks || []), ...(c.shop_needs || []).map((n) => `Needs: ${n}`), ...(c.tools_needed || []).map((n) => `Needs: ${n.description}`)]));
  }
  blocks.push(paragraph(`Choice source: ${text(plan.workholding?.source)}`), heading("Every decision"));
  blocks.push(table(["Id · stage · subject", "Decision", "Rule · source"], (plan.decisions || []).map((d) => {
    const alternatives = typeof d.chosen === "string" ? [...new Set((d.considered || []).map((c) => c.candidate).filter((c) => c !== d.chosen))] : [];
    const choice = alternatives.length ? ` Chose ${d.chosen} over ${alternatives.join(", ")}.` : "";
    const source = `${text(d.source)}${d.source && d.source !== "rule" && d.reason ? `: ${d.reason}` : ""}`;
    return row([`${d.id} · ${STAGES[d.stage] || d.stage} · ${text(d.subject)}`, `${d.says}${choice}${d.superseded_by ? ` Replaced by ${d.superseded_by}.` : ""}`, `${d.rule} · ${source}`]);
  }), "decisions"));
  blocks.push(heading("Operation detail"), table(["Operation", "Chosen tool / reason", "Notes and ISO 14649 rationale"], [...ops.values()].map((o) => {
    const choices = [...toolChoiceRows(o, decisions)].filter(([id, choice]) => choice.chosen && id === o.tool);
    return [o.id, choices.map(([id, choice]) => `${id}: ${choice.reason}`).join("; ") || "—", [o.tool_note, o.notes, o.iso14649?.note].filter(Boolean).map(value).join("\n") || "—"];
  }), "operation-detail"));
  blocks.push(heading("Rules cited"), table(["Rule id", "Rule text", "Sources"], [...new Set((plan.decisions || []).map((d) => d.rule))].map((id) => {
    const r = rules.get(id);
    return row([text(id), r ? text(r.rationale) : "Rule text unavailable.", r ? text((r.sources || []).map(describeSource).join("; ")) : "—"]);
  }), "rules"));
  const headline = programs.map((p) => `${p.setup}: ${p.material ? "compared with the part, no gouge found." : "not compared with the part."}`).join(" ");
  blocks.push(heading("Camlib checks and limits"), paragraph(headline || text(data.error || "—")), list(check.items));
  blocks.push(table(["Setup", "Material check", "Stock-contact / uncertain motions"], plan.setups.map((s) => {
    const p = programs.find((p) => p.setup === s.id);
    // The count is the program's; a run that stopped gives out no programs, only its log.
    const uncertain = p?.uncertain_motions > 0 ? [`note: setup ${s.id}: camlib's stock-contact check could not clear ${p.uncertain_motions} motions`] : [];
    const log = uncertain.concat((Array.isArray(data.log) ? data.log : []).filter((line) => line.includes(`setup ${s.id}:`) && (!p && line.includes("stock-contact") || line.includes("per-motion gouge proof"))));
    return row([s.id, p ? p.material ? `Compared with the part; no gouge found. Complete: ${text(p.material.complete)}` : "Not compared with the part." : "not run", log.join("\n") || "—"]);
  }), "checks"));
  blocks.push(paragraph("Uncertain-motion counts are only available where recorded in the camlib log; an absent count is —. Holding force and support flatness are not simulated."));
  if (data.log?.length) blocks.push(heading("Camlib log", 3), list((Array.isArray(data.log) ? data.log : [data.log]).map(text)));
  return { title: text(data.name), appendixStart, blocks };
}

export function toolChoiceRows(op, decisions) {
  const choices = (op.because || []).map((id) => decisions.get(id))
    .filter((d) => d?.stage === "tools" && !d.superseded_by
      && (d.considered?.length || (typeof d.chosen === "string" && d.chosen === op.tool)));
  if (!choices.length) return new Map();
  const rows = new Map();
  for (const d of choices) {
    const considered = d.considered || [];
    if (typeof d.chosen === "string") {
      const others = considered.filter((c) => c.candidate !== d.chosen);
      const preference = ["the largest", "the smallest", "the first in the library"].find((p) =>
        others.some((c) => c.rejected === `qualifies, but ${d.chosen} is preferred (${p})`));
      const only = others.length && others.every((c) => c.rejected?.trim() && !c.qualifies);
      const reason = preference ? `${preference} that qualifies` : only ? "the only one that qualifies" : d.says || "chosen";
      rows.set(d.chosen, { chosen: true, reason });
    }
    for (const c of considered) {
      if (!rows.get(c.candidate)?.chosen) rows.set(c.candidate, { chosen: false, reason: c.rejected });
    }
  }
  return rows;
}
