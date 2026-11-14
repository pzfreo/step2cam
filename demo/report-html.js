import { escapeHtml } from './report.js';

// Tokens mirror the approved mock-up; the sheet follows the system theme.
export const reportCss = `
:root{color-scheme:light dark;--bg:#fbfaf7;--ink:#1d1f22;--muted:#5f646b;--line:#d9d6cf;--jaw:#c9ccd1;--jawE:#7d838b;--par:#e3d7b8;--parE:#9c8650;--stock:#8a8f96;--part:#1d1f22;--partF:#eef1f5;--pick:#c2410c;--bed:#e7e5e0;--card:#fff}
@media(prefers-color-scheme:dark){:root{--bg:#16181b;--ink:#e8e6e1;--muted:#a3a7ad;--line:#33373c;--jaw:#3a3f46;--jawE:#8a919a;--par:#4a4230;--parE:#c7ad6b;--stock:#8e949b;--part:#e8e6e1;--partF:#23272c;--pick:#fb923c;--bed:#262a2f;--card:#1d2024}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 system-ui,sans-serif}main{max-width:1000px;margin:auto;padding:24px;background:var(--card)}h1{font-size:22px}h2{font-size:18px}h3{font-size:15px}p{white-space:pre-line}table{width:100%;border-collapse:collapse;font-size:12px;margin:8px 0 16px}td,th{border:1px solid var(--line);padding:5px;text-align:left;white-space:pre-line;overflow-wrap:anywhere}.setup-zero td{border:0;padding:0;font-size:14px}.setup-zero td:first-child{width:60px}.page-break{break-before:page;border-top:1px solid var(--line);margin-top:22px}.setup-diagram{margin:16px 0}.setup-diagram figure{margin:12px 0}.setup-diagram figcaption,.diagram-note,.diagram-key{font-size:12px;color:var(--muted)}.setup-diagram svg{display:block;width:min(100%,720px);height:auto}.diagram-key{display:flex;flex-wrap:wrap;gap:6px 16px}.diagram-key i{display:inline-block;width:14px;height:9px;border:1px solid;vertical-align:-1px;margin-right:5px}
svg .jaw{fill:var(--jaw);stroke:var(--jawE)}svg .parallel{fill:var(--par);stroke:var(--parE)}svg .bed{fill:var(--bed);stroke:none}svg .part{fill:var(--partF);stroke:none}svg .edge{fill:none;stroke:var(--part)}svg .stock{fill:none;stroke:var(--stock);stroke-dasharray:2 1.2}svg .hidden{fill:none;stroke:var(--muted);stroke-dasharray:1.6 1}svg .pickup,svg .axis,svg .leader{fill:none;stroke:var(--pick)}svg .zero{fill:var(--card);stroke:var(--pick)}svg .stop{fill:var(--jaw);stroke:var(--pick)}svg .dimension{fill:none;stroke:var(--ink)}svg text{font-family:system-ui,sans-serif;fill:var(--ink);stroke:none!important}svg text.dimension{fill:var(--ink)}svg .zero-fill{fill:var(--pick);stroke:none}svg text.pickup{fill:var(--pick)}svg text.tag{fill:var(--muted)}
@media print{main{max-width:none;padding:0}body{background:white;color:black} .page-break{border:0}}
`;

export function diagramSvg(view) {
  const [x0, y0, x1, y1] = view.bounds, u = view.unit;
  const shapes = view.shapes.map(s => {
    const cls = `class="${s.class}"`, width = (s.class === 'pickup' || s.class === 'axis' ? 0.65 : 0.35) * u;
    if (s.type === 'text') return `<text ${cls} x="${s.x}" y="${-s.y}" font-size="${3.4 * u}" text-anchor="${s.anchor}">${escapeHtml(s.text)}</text>`;
    const dash = s.class === 'stock' ? [2 * u, 1.2 * u] : s.class === 'hidden' ? [1.6 * u, u] : null;
    const style = `${cls} stroke-width="${width}"${dash ? ` style="stroke-dasharray:${dash.join(' ')}"` : ''}`;
    if (s.type === 'rect') return `<rect ${style} x="${s.x}" y="${-s.y - s.height}" width="${s.width}" height="${s.height}"/>`;
    if (s.type === 'circle') return `<circle ${style} cx="${s.x}" cy="${-s.y}" r="${s.radius}"/>`;
    return `<${s.type === 'polygon' ? 'polygon' : 'polyline'} ${style} points="${s.points.map(p => `${p[0]},${-p[1]}`).join(' ')}"/>`;
  }).join('');
  return `<svg viewBox="${x0} ${-y1} ${x1 - x0} ${y1 - y0}" role="img" aria-label="${escapeHtml(view.caption)}">${shapes}</svg>`;
}
export function diagramHtml(diagram) {
  const swatches = { jaw: 'background:var(--jaw);border-color:var(--jawE)', parallel: 'background:var(--par);border-color:var(--parE)', part: 'background:var(--partF);border-color:var(--part)', stock: 'border:1px dashed var(--stock)', hidden: 'border:1px dashed var(--muted)', pickup: 'border-color:var(--pick)' };
  return `<div class="setup-diagram">${diagram.warnings.map(w => `<p>${escapeHtml(w)}</p>`).join('')}${diagram.views.map(v => `<figure><figcaption>${escapeHtml(v.caption)}</figcaption>${diagramSvg(v)}</figure>`).join('')}<div class="diagram-key">${diagram.legend.map(([cls, label]) => `<span><i style="${swatches[cls]}"></i>${escapeHtml(label)}</span>`).join('')}</div>${diagram.note ? `<p class="diagram-note">${escapeHtml(diagram.note)}</p>` : ''}</div>`;
}
export function renderReportHtml(report) {
  const html = report.blocks.map(b => {
    if (b.type === 'heading') return `<h${b.level}>${escapeHtml(b.text)}</h${b.level}>`;
    if (b.type === 'paragraph') return `<p>${escapeHtml(b.text)}</p>`;
    if (b.type === 'list') return `<ul>${b.items.map(i => `<li>${escapeHtml(i)}</li>`).join('')}</ul>`;
    if (b.type === 'page-break') return '<div class="page-break"></div>';
    if (b.type === 'setup-diagram') return diagramHtml(b);
    if (b.type === 'table') return `<table class="${escapeHtml(b.id)}">${b.headers.length ? `<thead><tr>${b.headers.map(h => `<th>${escapeHtml(h)}</th>`).join('')}</tr></thead>` : ''}<tbody>${b.rows.map(r => `<tr>${r.map(c => `<td>${escapeHtml(c)}</td>`).join('')}</tr>`).join('')}</tbody></table>`;
    throw new Error(`Unknown report block: ${b.type}`);
  }).join('');
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escapeHtml(report.title)} — Setup sheet</title><style>${reportCss}</style></head><body><main>${html}</main></body></html>`;
}
