// Shared page wording and renderer-neutral report content. No DOM or PDF dependency.

// specify-core's descriptions of the part as a whole, not features a person would point at.
export const NOT_FEATURES = new Set(["hole_patterns", "step_levels", "risers", "section_recesses", "plates", "thin_wall_bodies", "sheet_metal_bodies", "turned_bodies", "circular_face_patterns"]);

export const trim = (x) => x == null ? "—" : String(Math.round(Number(x) * 10) / 10);

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
    say(`at least ${number(residual.volume_mm3[0])} mm³ of stock is still there. That includes the web left around the part, and anything a later setup cuts.`);
    if (limits.before_threads) say(`threads are not compared, because they go beyond the bore the model shows.`);
  }
  if (unchecked.length) items.push("Not compared: only the setup with roughing holds the part's model in its position, so side holes and the like rely on the other checks.");
  return { headline: headline.join(" "), compact, items };
}

export function workholdingText(c) {
  const seconds = c.estimated_seconds == null ? null : Math.round(c.estimated_seconds);
  const duration = seconds == null ? "—" : `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
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
  return `Tool comes down −Z; zero on the ${setup.zero == null ? "—" : setup.zero === "stock" ? "stock" : "part's box"}, stock ${lo && hi ? hi.map((v, i) => format(v - lo[i])).join(" × ") : "—"} mm.`;
}

const value = (x) => x == null || x === "" ? "—" : typeof x === "object" ? JSON.stringify(x) : String(x);
const text = value;
export const reportLength = (x) => x == null ? "—" : String(Number(x) === 3.175 ? 3.175 : Number(Number(x).toFixed(2)));
const rate = (x) => x == null ? "—" : String(Math.round(x));
export function reportDuration(seconds) {
  const minutes = Math.round(seconds / 60);
  return `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")} min`;
}
// Keep the page's established wording; only the machinist's sheet simplifies it.
export function sheetWords(input) {
  let s = value(input);
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
        try { JSON.parse(part); s = s.slice(0, start) + s.slice(end + 1); start--; } catch { /* ordinary bracketed prose */ }
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
  return s.replace(/\b[A-Za-z][A-Za-z0-9]*(?:_[A-Za-z0-9]+)+\b/g, (id) => id.replaceAll("_", " "))
    .replace(/-?\d+\.\d+/g, (n) => reportLength(Number(n))).replace(/, offsets\s*; measured side contact\s*\./g, ".").replace(/:\s*;/g, ";").replace(/ {2,}/g, " ").trim();
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
  const missing = [...needs.map((n) => ({ ...n, context: "Current plan" })),
    ...(plan.workholding?.candidates || []).flatMap((c) => (c.tools_needed || []).map((n) => ({ ...n, context: c.label || c.id })))];
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
    text(machine?.name || data.library?.machine), text(date), text(plan.material?.used), stockSize(plan.setups[0]?.stock),
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
    blocks.push(pageBreak(), heading(`SETUP — ${setup.id}`, 1),
      paragraph(sheetWords(`Workholding: ${text(holding)} · grip depth ${reportLength(grip?.height_mm ?? setup.clamping?.grip_height_mm)} mm${setup.flip_seating ? " · flip onto parallels" : ""}${grip?.finished_wall ? " · soft jaws advised" : ""}`)),
      paragraph(`Work zero: ${setupZero(setup, reportLength)}`));
    const rows = setup.operations.map((op, i) => {
      const id = `${setup.id}-${i + 1}-${op.kind}`, o = ops.get(id), t = op.technology || {};
      const notes = [];
      for (const [id, choice] of toolChoiceRows(op, decisions)) if (choice.chosen && id === op.tool) notes.push(`Tool choice: ✓ ${id} — ${choice.reason}`);
      if (t.peck_mode != null) notes.push(`Peck mode: ${t.peck_mode}`);
      if (t.peck_mm != null) notes.push(`Peck depth: ${reportLength(t.peck_mm)} mm`);
      if (op.tool_note) notes.push(op.tool_note);
      if (op.notes) notes.push(value(op.notes));
      if (op.iso14649?.note) notes.push(op.iso14649.note);
      const old = tools.find((t) => t.id === previousTool), next = tools.find((t) => t.id === op.tool);
      if (manualCollets && previousTool && previousTool !== op.tool) {
        if (old?.shank_diameter != null && next?.shank_diameter != null) {
          if (old.shank_diameter !== next.shank_diameter) notes.push(`Manual collet change: ${previousTool} → ${op.tool} (${reportLength(old.shank_diameter)} → ${reportLength(next.shank_diameter)} mm shank).`);
        } else notes.push(`Check manual collet change: ${previousTool} → ${op.tool}; shank size —.`);
      }
      if (op.tool) previousTool = op.tool;
      notes.push(...warnings.filter((d) => op.because?.includes(d.id)).map((d) => warningText(m, d, true)));
      // Notes stay in the operation's row so the PDF never separates them.
      return row([text(i + 1), sheetWords(o?.name || operationName(op, setup, plan)) + (notes.length ? "\n" + notes.map(sheetWords).join("\n") : ""), toolLabel(op.tool), rate(t.spindle_rpm), rate(t.feed_mm_min), rate(t.plunge_mm_min), `${reportLength(t.stepdown_mm)} / ${reportLength(t.stepover_mm)}`, o?.time ? describeTime(Math.round(o.time)) : "not run"]);
    });
    blocks.push(table(["#", "Operation / notes", "Tool", "RPM", "Feed mm/min", "Plunge mm/min", "↓ / ↔ mm", "Time est."], rows, "operations"));
  }
  blocks.push(heading("NOT CUT"), table(["Feature", "Reason", "Tool that would cut it"], (plan.skipped || []).map((s) => row([
    sheetWords(`${fs.get(s.feature)?.name || s.feature} (${s.feature})`), sheetWords(s.reason),
    sheetWords(needs.filter((n) => n.features?.includes(s.feature)).map((n) => n.description).join("; ")),
  ])).concat([...fs.values()].filter((f) => f.notCut && !plan.skipped?.some((s) => s.feature === f.id)).map((f) => row([sheetWords(f.name), sheetWords(f.note), "—"]))), "not-cut"));
  blocks.push(heading("WARNINGS"));
  const groups = new Map();
  for (const d of warnings) {
    const label = rules.get(d.rule)?.warning_label || "plan needs checking";
    if (!groups.has(label)) groups.set(label, []);
    groups.get(label).push(sheetWords(warningText(m, d)));
  }
  for (const [label, items] of groups) blocks.push(heading(sheetWords(label), 3), list(items));
  const extra = (plan.warnings || []).filter((w) => !warnings.some((d) => d.says === w));
  if (extra.length) blocks.push(list(extra.map(sheetWords)));
  if (!warnings.length && !extra.length) blocks.push(paragraph("—"));
  const refused = (plan.decisions || []).filter((d) => d.rule?.startsWith("execution.refused@") && d.outcome === "removed");
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
  blocks.push(heading("Rules cited"), table(["Rule id", "Rule text", "Sources"], [...new Set((plan.decisions || []).map((d) => d.rule))].map((id) => {
    const r = rules.get(id);
    return row([text(id), r ? text(r.rationale) : "Rule text unavailable.", r ? text((r.sources || []).map(describeSource).join("; ")) : "—"]);
  }), "rules"));
  const headline = programs.map((p) => `${p.setup}: ${p.material ? "compared with the part, no gouge found." : "not compared with the part."}`).join(" ");
  blocks.push(heading("Camlib checks and limits"), paragraph(headline || text(data.error || "—")), list(check.items));
  blocks.push(table(["Setup", "Material check", "Stock-contact / uncertain motions"], plan.setups.map((s) => {
    const p = programs.find((p) => p.setup === s.id);
    const log = (Array.isArray(data.log) ? data.log : []).filter((line) => line.includes(`setup ${s.id}:`) && (line.includes("stock-contact") || line.includes("per-motion gouge proof")));
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
      const only = others.length && others.every((c) => c.rejected?.trim() && !c.rejected.startsWith("qualifies, but"));
      const reason = preference ? `${preference} that qualifies` : only ? "the only one that qualifies" : d.says || "chosen";
      rows.set(d.chosen, { chosen: true, reason });
    }
    for (const c of considered) {
      if (!rows.get(c.candidate)?.chosen) rows.set(c.candidate, { chosen: false, reason: c.rejected });
    }
  }
  return rows;
}
