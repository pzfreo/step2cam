// Shared plan wording and a DOM-independent printable report. All interpolated data is escaped.

// specify-core's descriptions of the part as a whole, not features a person would point at.
export const NOT_FEATURES = new Set(["hole_patterns", "step_levels", "risers", "section_recesses", "plates", "thin_wall_bodies", "sheet_metal_bodies", "turned_bodies", "circular_face_patterns"]);

export const trim = (x) => x == null ? "—" : String(Math.round(Number(x) * 10) / 10);

export function featureName(f) {
  const r = f.record || {};
  switch (f.family) {
    case "holes": return `⌀${trim(r.diameter)} hole`;
    case "slots": return `Slot, ${trim(r.width)} wide`;
    case "pockets": return r.open_sign ? `Notch, ${trim(r.width)} wide × ${trim(r.depth)} deep` : `Pocket, ${trim(r.width)} wide × ${trim(r.depth)} deep`;
    case "v_channels": return `V-channel, ${trim(r.width)} wide × ${trim(r.depth)} deep`;
    case "polygonal_bosses": return r.side_count === 6 ? `Hex boss, ${trim(r.across_flats)} across flats` : `${r.side_count ?? "—"}-sided boss`;
    case "fillets": return r.side === "convex" ? `r${trim(r.radius)} edge round` : `r${trim(r.radius)} fillet`;
    default: return (f.family || "—").replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
  }
}

export function operationName(op, setup, plan) {
  const where = plan.setups.length > 1 ? ` (${setup.id})` : "";
  if (op.kind === "spot_drill") return "Spot drill" + where;
  if (op.kind === "facing") return "Facing" + where;
  if (op.kind === "roughing") return "Roughing" + where;
  if (op.kind === "counterbore") return `⌀${op.diameter ?? "—"} counterbore` + where;
  if (op.kind === "thread") return `${op.spec ?? "—"} thread` + where;
  if (op.kind === "slot") return `${op.width ?? "—"} × ${op.length ?? "—"} slot` + where;
  if (op.kind === "notch") return `${trim(op.width)} × ${trim(op.length)} ${op.open ? "notch" : "slot"}` + where;
  if (op.kind === "v_channel") return `V-channel, ${trim(op.width)} wide` + where;
  if (op.kind === "flats") return "Finish flats" + where;
  if (op.kind === "wall_finish") return "Finish outside wall" + where;
  if (op.kind === "edge_chamfer") return (op.radius == null ? `${op.width ?? "—"} mm 45° edge chamfer` : `${op.width ?? "—"} chamfer on r${op.radius ?? "—"} edges`) + where;
  return `⌀${op.diameter ?? "—"} ${op.bottom === "through" ? "through" : op.bottom === "drill_point" ? "drill point" : op.bottom}` + where;
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

export function setupZero(setup) {
  const lo = setup.stock?.min, hi = setup.stock?.max;
  return `Tool comes down −Z; zero on the ${setup.zero == null ? "—" : setup.zero === "stock" ? "stock" : "part's box"}, stock ${lo && hi ? hi.map((v, i) => trim(v - lo[i])).join(" × ") : "—"} mm.`;
}

const value = (x) => x == null || x === "" ? "—" : typeof x === "object" ? JSON.stringify(x) : String(x);
const text = (x) => escapeHtml(value(x));
const list = (items) => items.length ? `<ul>${items.map((x) => `<li>${x}</li>`).join("")}</ul>` : "<p>—</p>";
const table = (heads, rows, className = "") => `<table${className ? ` class="${className}"` : ""}><thead><tr>${heads.map((h) => `<th>${h}</th>`).join("")}</tr></thead><tbody>${rows.length ? rows.join("") : `<tr><td colspan="${heads.length}">—</td></tr>`}</tbody></table>`;
const row = (cells) => `<tr>${cells.map((c) => `<td>${c}</td>`).join("")}</tr>`;

/** A self-contained document, with no page DOM or browser dependencies.
 * Maps may supply the page's move-derived operation times and feature annotations.
 * Without them, times are calculated from the same program moves and feed rates.
 */
export function reportHtml(data, { rules = new Map(), library = {}, operations, features, estimatedSeconds, date = new Date().toLocaleDateString("en-GB") } = {}) {
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
  const fs = features || new Map((data.analysis?.features || []).filter((f) => !NOT_FEATURES.has(f.family)).map((f) => [f.id, { ...f, name: featureName(f) }]));
  const ops = operations || new Map(plan.setups.flatMap((s) => s.operations.map((o, i) => {
    const id = `${s.id}-${i + 1}-${o.kind}`;
    return [id, { ...o, id, setup: s, name: operationName(o, s, plan), time: times.get(id) || 0 }];
  })));
  const m = { data, operations: ops, features: fs };
  const warnings = planWarnings(m);
  const check = checkText(programs);
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
  const sections = [];
  sections.push(`<h1>SETUP SHEET</h1><h2>${text(data.name)}</h2><p class="draft">Draft — check before cutting</p>`);
  sections.push(table(["Machine", "Date", "Material", "Stock size (mm)", "Feed-rate estimate"], [row([
    text(machine?.name || data.library?.machine), text(date), text(plan.material?.used),
    text(plan.setups[0]?.stock ? plan.setups[0].stock.max.map((v, i) => trim(v - plan.setups[0].stock.min[i])).join(" × ") : null),
    programs.length ? text(describeTime(estimatedSeconds ?? total)) : "—",
  ])]));
  sections.push(`<p>Camlib: ${text(data.error ? `could not run the plan: ${data.error}` : programs.length ? check.compact : "—")}</p>`);
  if (data.notice) sections.push(`<p>${text(data.notice)}</p>`);
  sections.push("<h2>TOOLS — load before starting</h2>");
  if (machine?.spindle?.manual_collets) sections.push("<p>Manual collets: check the collet at every tool change. Changes of recorded shank size are flagged below.</p>");
  sections.push(table(["T / tool id", "Type / ⌀ mm", "Flute length mm", "Stickout / overall mm", "RPM", "Feed mm/min"], used.map((id) => {
    const t = tools.find((t) => t.id === id);
    const uses = [...ops.values()].filter((o) => o.tool === id);
    const settings = (key) => [...new Set(uses.map((o) => value(o.technology?.[key])))].map(text).join(" / ");
    return row([toolLabel(id), `${text(t?.type)} / ${text(t?.diameter)}`, text(t?.flute_length), `${text(t?.stickout)} / ${text(t?.overall_length)}`, settings("spindle_rpm"), settings("feed_mm_min")]);
  })));
  sections.push("<h2>Tools needed but not in the library</h2>", list([...missingByTool].map(([description, n]) => `${text(description)} — needed by ${[...n.contexts].map(text).join("; ")}${n.features.size ? ` — ${[...n.features].map((id) => text(fs.get(id)?.name || id)).join(", ")}` : ""}`)));
  let previousTool;
  for (const setup of plan.setups) {
    const grip = chosen?.grips?.find((g) => g.setup === setup.id);
    const holding = chosen ? workholdingText(chosen).label : null;
    sections.push(`<section class="setup"><h1>SETUP — ${text(setup.id)}</h1><p><b>Workholding:</b> ${text(holding)} · grip depth ${text(grip?.height_mm ?? setup.clamping?.grip_height_mm)} mm${setup.flip_seating ? " · flip onto parallels" : ""}${grip?.finished_wall ? " · soft jaws advised" : ""}</p><p><b>Work zero:</b> ${text(setupZero(setup))}</p>`);
    const rows = setup.operations.map((op, i) => {
      const id = `${setup.id}-${i + 1}-${op.kind}`, o = ops.get(id), t = op.technology || {};
      const notes = [];
      const choices = toolChoiceRows(op, decisions);
      for (const [id, choice] of choices) if (choice.chosen && id === op.tool) notes.push(`Tool choice: ✓ ${text(id)} — ${text(choice.reason)}`);
      if (t.peck_mode != null) notes.push(`Peck mode: ${text(t.peck_mode)}`);
      if (t.peck_mm != null) notes.push(`Peck depth: ${text(t.peck_mm)} mm`);
      if (op.tool_note) notes.push(text(op.tool_note));
      if (op.notes) notes.push(text(op.notes));
      if (op.iso14649?.note) notes.push(text(op.iso14649.note));
      const old = tools.find((t) => t.id === previousTool), next = tools.find((t) => t.id === op.tool);
      if (machine?.spindle?.manual_collets && previousTool && previousTool !== op.tool) {
        // Shank diameter is the collet fit; cutting diameter is not a substitute.
        if (old?.shank_diameter != null && next?.shank_diameter != null) {
          if (old.shank_diameter !== next.shank_diameter) notes.push(`Manual collet change: ${text(previousTool)} → ${text(op.tool)} (${text(old.shank_diameter)} → ${text(next.shank_diameter)} mm shank).`);
        } else notes.push(`Check manual collet change: ${text(previousTool)} → ${text(op.tool)}; shank size —.`);
      }
      if (op.tool) previousTool = op.tool;
      notes.push(...warnings.filter((d) => op.because?.includes(d.id)).map((d) => text(warningText(m, d, true))));
      const main = row([text(i + 1), text(o?.name || operationName(op, setup, plan)), toolLabel(op.tool), text(t.spindle_rpm), text(t.feed_mm_min), text(t.plunge_mm_min), `${text(t.stepdown_mm)} / ${text(t.stepover_mm)}`, o?.time ? text(describeTime(o.time)) : "not run"]);
      return `<tbody>${main}${notes.length ? `<tr><td colspan="8" class="notes">${notes.join("<br>")}</td></tr>` : ""}</tbody>`;
    });
    // One tbody per operation keeps its notes with the cutting data when printing.
    sections.push(`<table class="operations"><thead><tr>${["#", "Operation", "Tool", "RPM", "Feed mm/min", "Plunge mm/min", "↓ / ↔ mm", "Time est."].map((h) => `<th>${h}</th>`).join("")}</tr></thead>${rows.join("")}</table></section>`);
  }
  sections.push("<h2>NOT CUT</h2>", table(["Feature", "Reason", "Tool that would cut it"], (plan.skipped || []).map((s) => row([
    `${text(fs.get(s.feature)?.name || s.feature)} (${text(s.feature)})`, text(s.reason),
    text(needs.filter((n) => n.features?.includes(s.feature)).map((n) => n.description).join("; ")),
  ])).concat([...fs.values()].filter((f) => f.notCut && !plan.skipped?.some((s) => s.feature === f.id)).map((f) => row([text(f.name), text(f.note), "—"])))));
  sections.push("<h2>WARNINGS</h2>");
  const groups = new Map();
  for (const d of warnings) {
    const label = rules.get(d.rule)?.warning_label || "plan needs checking";
    if (!groups.has(label)) groups.set(label, []);
    groups.get(label).push(text(warningText(m, d)));
  }
  for (const [label, items] of groups) sections.push(`<h3>${text(label)}</h3>`, list(items));
  // Preserve warning strings even if a future/older plan has no matching trace entry.
  const extra = (plan.warnings || []).filter((w) => !warnings.some((d) => d.says === w));
  if (extra.length) sections.push(list(extra.map(text)));
  if (!warnings.length && !extra.length) sections.push("<p>—</p>");
  const refused = (plan.decisions || []).filter((d) => d.rule?.startsWith("execution.refused@") && d.outcome === "removed");
  if (refused.length) sections.push("<h3>Refused by camlib</h3>", list(refused.map((d) => `${text(d.subject)}: ${text(d.says)}`)));
  sections.push('<section class="appendix"><h1>APPENDIX — FULL REPORT</h1><h2>Recognised features</h2>', table(["Id", "Feature", "What happens to it"], [...fs.values()].map((f) => {
    const cutBy = [...ops.values()].filter((o) => o.features?.includes(f.id));
    const disposition = f.note || (cutBy.length ? [...new Set(cutBy.map((o) => o.name))].join("; ")
      : (plan.skipped || []).some((s) => s.feature === f.id) || fs.get(f.id)?.notCut ? "not cut"
      : (plan.covered_by_roughing || []).includes(f.id) ? "covered by roughing" : "not cut");
    return row([text(f.id), text(f.name || featureName(f)), text(disposition)]);
  }), "features"));
  sections.push("<h2>Workholding options and consequences</h2>");
  sections.push(list((plan.workholding?.candidates || []).map((c) => {
    const { label, summary } = workholdingText(c);
    return `<b>${c.id === plan.workholding.chosen ? "✓ " : ""}${text(label)}</b> — ${text(summary)}<br>${text(c.comparison)}${list([...(c.risks || []), ...(c.shop_needs || []).map((n) => `Needs: ${n}`), ...(c.tools_needed || []).map((n) => `Needs: ${n.description}`)].map(text))}`;
  })));
  sections.push(`<p>Choice source: ${text(plan.workholding?.source)}</p><h2>Every decision</h2>`);
  sections.push(table(["Id · stage · subject", "Decision", "Rule · source"], (plan.decisions || []).map((d) => {
    const alternatives = typeof d.chosen === "string" ? [...new Set((d.considered || []).map((c) => c.candidate).filter((c) => c !== d.chosen))] : [];
    const choice = alternatives.length ? ` Chose ${text(d.chosen)} over ${alternatives.map(text).join(", ")}.` : "";
    const source = `${text(d.source)}${d.source && d.source !== "rule" && d.reason ? `: ${text(d.reason)}` : ""}`;
    return row([`${text(d.id)} · ${text(STAGES[d.stage] || d.stage)} · ${text(d.subject)}`,
      `${text(d.says)}${choice}${d.superseded_by ? ` Replaced by ${text(d.superseded_by)}.` : ""}`,
      `${text(d.rule)} · ${source}`]);
  }), "decisions"));
  sections.push("<h2>Rules cited</h2>", table(["Rule id", "Rule text", "Sources"], [...new Set((plan.decisions || []).map((d) => d.rule))].map((id) => {
    const r = rules.get(id);
    return row([text(id), r ? text(r.rationale) : "Rule text unavailable.", r ? text((r.sources || []).map(describeSource).join("; ")) : "—"]);
  }), "rules"));
  sections.push("<h2>Camlib checks and limits</h2>", `<p>${check.headline || text(data.error || "—")}</p>`, list(check.items.map(text)));
  sections.push(table(["Setup", "Material check", "Stock-contact / uncertain motions"], plan.setups.map((s) => {
    const p = programs.find((p) => p.setup === s.id);
    return row([text(s.id), p ? p.material ? `Compared with the part; no gouge found. Complete: ${text(p.material.complete)}` : "Not compared with the part." : "not run",
      list((Array.isArray(data.log) ? data.log : []).filter((line) => line.includes(`setup ${s.id}:`) && (line.includes("stock-contact") || line.includes("per-motion gouge proof"))).map(text))]);
  })));
  sections.push("<p>Uncertain-motion counts are only available where recorded in the camlib log; an absent count is —. Holding force and support flatness are not simulated.</p>");
  if (data.log?.length) sections.push("<h3>Camlib log</h3>", list((Array.isArray(data.log) ? data.log : [data.log]).map(text)));
  sections.push("</section>");
  // JSON quoting provides a safe CSS string, including filenames with quotes/newlines.
  const pageName = JSON.stringify(String(data.name || "—")).replaceAll("<", "\\3c ");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="color-scheme" content="light"><title>${text(data.name)} — Setup sheet</title><style>
@page { size: A4 portrait; margin: 17mm 12mm; @top-left { content: ${pageName}; font: 9pt sans-serif; } @bottom-right { content: "Page " counter(page) " of " counter(pages); font: 9pt sans-serif; } }
* { box-sizing: border-box; } html { color-scheme: light; background: white; color: #111; } body { font: 10pt/1.35 Arial, sans-serif; margin: 0 auto; max-width: 190mm; padding: 5mm; } h1 { font-size: 19pt; } h2 { font-size: 13pt; margin: 5mm 0 2mm; } h3 { font-size: 11pt; } h1,h2,h3 { break-after: avoid; } p { margin: 2mm 0; } table { width: 100%; border-collapse: collapse; margin: 3mm 0; table-layout: fixed; } th,td { text-align: left; vertical-align: top; border: 1px solid #bbb; padding: 1.5mm; overflow-wrap: anywhere; } th { background: #eee; } tr, .operations tbody { break-inside: avoid; page-break-inside: avoid; } thead { display: table-header-group; } .setup,.appendix { break-before: page; page-break-before: always; } .draft { font-weight: bold; border: 2px solid #555; padding: 2mm; } ul { padding-left: 5mm; } .notes { background: #f5f5f5; }
.appendix { line-height: 1.2; } .appendix th,.appendix td { padding: 1mm; }
.decisions th:first-child { width: 22%; } .decisions th:nth-child(2) { width: 53%; } .decisions th:last-child { width: 25%; }
.rules th:first-child { width: 20%; } .rules th:nth-child(2) { width: 60%; } .rules th:last-child { width: 20%; }
.features th:first-child { width: 20%; } .features th:nth-child(2) { width: 35%; } @media print { body { max-width: none; padding: 0; } }
</style></head><body>${sections.join("\n")}</body></html>`;
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
