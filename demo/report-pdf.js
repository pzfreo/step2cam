// Loaded only by the setup-sheet click. The report model owns all content.
// jsPDF 2.5.2 is intentional: reported addImage/html CVE paths are unused here;
// this renderer calls vector primitives, text and AutoTable, never html or addImage.
const FONT_BASE = "https://cdn.jsdelivr.net/npm/dejavu-fonts-ttf@2.37.3/ttf/";
let dependencies;
const binary = (bytes) => {
  let result = "";
  for (let i = 0; i < bytes.length; i += 8192) result += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(result);
};
async function font(name) {
  const response = await fetch(FONT_BASE + name, { signal: AbortSignal.timeout(30000) });
  if (!response.ok) throw new Error(`font download failed (${response.status})`);
  return binary(new Uint8Array(await response.arrayBuffer()));
}
async function loadDependencies() {
  if (!dependencies) dependencies = Promise.all([
    import("jspdf"), import("jspdf-autotable"), font("DejaVuSans.ttf"), font("DejaVuSans-Bold.ttf"),
  ]).then(([{ jsPDF }, { default: autoTable }, regular, bold]) => ({ jsPDF, autoTable, fonts: { regular, bold } }))
    .catch((error) => { dependencies = null; throw error; });
  return dependencies;
}

// Wrap long identifiers/JSON at existing separators, never through a word or
// number. Added spaces do not change identifiers when extracted without whitespace.
function readableBreaks(input, width, measure) {
  return String(input).split(/(\s+)/).map((token) => {
    if (!token.trim() || measure(token) <= width) return token;
    const parts = token.replace(/([._/@:,\-{}\[\]()"';=])/g, "$1 ");
    for (const part of parts.split(/\s+/)) {
      if (measure(part) > width) throw new Error("Report column too narrow for a complete word");
    }
    return parts;
  }).join("");
}

// Fixed print colours, corresponding to the light-theme CSS tokens.
const diagramColours = {
  jaw: [[125, 131, 139], [201, 204, 209]], parallel: [[156, 134, 80], [227, 215, 184]],
  bed: [null, [231, 229, 224]], part: [null, [238, 241, 245]],
  edge: [[29, 31, 34], null], hidden: [[95, 100, 107], null], stock: [[138, 143, 150], null],
  leader: [[194, 65, 12], null], pickup: [[194, 65, 12], null], axis: [[194, 65, 12], null], zero: [[194, 65, 12], [255, 255, 255]],
  'zero-fill': [null, [194, 65, 12]], stop: [[194, 65, 12], [201, 204, 209]], dimension: [[29, 31, 34], null], tag: [[95, 100, 107], null],
};
export function renderDiagramPdf(doc, view, left, top, width, maxHeight) {
  const [x0, y0, x1, y1] = view.bounds;
  const scale = Math.min(width / (x1 - x0), maxHeight / (y1 - y0));
  const x = n => left + (n - x0) * scale, y = n => top + (y1 - n) * scale;
  for (const s of view.shapes) {
    const [stroke, fill] = diagramColours[s.class];
    if (stroke) doc.setDrawColor(...stroke);
    if (fill) doc.setFillColor(...fill);
    doc.setLineWidth((s.class === 'pickup' || s.class === 'axis' ? 0.65 : 0.35) * view.unit * scale);
    const dash = s.class === 'stock' ? [2, 1.2] : s.class === 'hidden' ? [1.6, 1] : [];
    doc.setLineDashPattern(dash.map(n => n * view.unit * scale), 0);
    const style = stroke && fill ? 'FD' : fill ? 'F' : 'S';
    if (s.type === 'text') {
      doc.setTextColor(...(stroke || [29, 31, 34]));
      doc.setFontSize(3.4 * view.unit * scale / 0.352778);
      doc.text(s.text, x(s.x), y(s.y), { align: s.anchor === 'middle' ? 'center' : s.anchor === 'end' ? 'right' : 'left' });
    } else if (s.type === 'rect') doc.rect(x(s.x), y(s.y + s.height), s.width * scale, s.height * scale, style);
    else if (s.type === 'circle') doc.circle(x(s.x), y(s.y), s.radius * scale, style);
    else if (s.type === 'polygon' || s.type === 'polyline') {
      const first = s.points[0];
      doc.lines(s.points.slice(1).map((p, i) => [(p[0] - s.points[i][0]) * scale, -(p[1] - s.points[i][1]) * scale]), x(first[0]), y(first[1]), [1, 1], style, s.type === 'polygon');
    }
  }
  doc.setLineDashPattern([], 0); doc.setTextColor(20); doc.setLineWidth(0.1);
  return (y1 - y0) * scale;
}

/** Render real Unicode text; dependencies can be injected for offline verification. */
export function renderReportPdf(report, { jsPDF, autoTable, fonts }) {
  const doc = new jsPDF({ orientation: "portrait", unit: "mm", format: "a4", compress: true, putOnlyUsedFonts: true });
  for (const [style, data] of [["normal", fonts.regular], ["bold", fonts.bold]]) {
    const file = `DejaVuSans-${style}.ttf`;
    doc.addFileToVFS(file, data);
    doc.addFont(file, "DejaVuSans", style);
  }
  doc.setFont("DejaVuSans", "normal");
  doc.setProperties({ title: `${report.title} — Setup sheet`, creator: "step2cam" });
  const left = 15, width = 180, top = 25, bottom = 277;
  let y = top;
  const newPage = () => { doc.addPage(); y = top; };
  function write(text, { size = 10, bold = false, gap = 2, indent = 0, keep = 0 } = {}) {
    doc.setFont("DejaVuSans", bold ? "bold" : "normal");
    doc.setFontSize(size);
    const lines = doc.splitTextToSize(readableBreaks(text, width - indent, (word) => doc.getTextWidth(word)), width - indent);
    const height = size * 0.352778 * 1.2;
    if (y + height * Math.min(lines.length, 2) + keep > bottom) newPage();
    for (const line of lines) {
      if (y + height > bottom) newPage();
      doc.text(line, left + indent, y + size * 0.352778);
      y += height;
    }
    y += gap;
  }
  for (const [index, block] of report.blocks.entries()) {
    if (block.type === "page-break") { if (y > top) newPage(); }
    else if (block.type === "heading") {
      y += 2;
      write(block.text, { size: block.level === 1 ? 17 : block.level === 2 ? 12 : 10, bold: true, gap: 3, keep: 16 });
    } else if (block.type === "paragraph") write(block.text);
    else if (block.type === "list") for (const item of block.items) write(`• ${item}`, { indent: 2, gap: 1 });
    else if (block.type === "setup-diagram") {
      // Keep both views together, including captions, warning and legend.
      const heights = block.views.map(v => Math.min(60, width * (v.bounds[3] - v.bounds[1]) / (v.bounds[2] - v.bounds[0])));
      const height = heights.reduce((a, b) => a + b, 0) + 34 + block.warnings.length * 6 + (block.note ? 6 : 0);
      if (y + height > bottom) newPage();
      for (const warning of block.warnings) write(warning, { size: 9 });
      block.views.forEach((view, i) => {
        write(view.caption, { size: 9, gap: 1 });
        y += renderDiagramPdf(doc, view, left, y, width, heights[i]) + 3;
      });
      // Swatches use the same print styles as the drawing.
      block.legend.forEach(([cls, label], i) => {
        const column = i % 3, row = Math.floor(i / 3), xx = left + column * 60;
        const [stroke, fill] = diagramColours[cls];
        doc.setDrawColor(...(stroke || [29, 31, 34]));
        if (fill) doc.setFillColor(...fill);
        doc.setLineDashPattern(cls === 'hidden' || cls === 'stock' ? [0.8, 0.5] : [], 0);
        doc.rect(xx, y + row * 5, 4, 2, fill ? 'FD' : 'S');
        doc.setFontSize(8); doc.setTextColor(70); doc.text(label, xx + 6, y + row * 5 + 2);
      });
      doc.setLineDashPattern([], 0); doc.setTextColor(20); y += 13;
      if (block.note) write(block.note, { size: 8 });
    }
    else if (block.type === "table") {
      // Give prose most of the width; narrow columns hold only cutting data.
      const zero = block.id === "setup-zero";
      const widths = {
        operations: [9, 47, 26, 17, 20, 20, 22, 19],
        decisions: [39.6, 95.4, 45], rules: [36, 108, 36], features: [36, 63, 81],
        "not-cut": [54, 81, 45], "operation-detail": [35, 45, 100], checks: [18, 55, 107],
        tools: [30, 34, 25, 37, 25, 29], header: [39, 23, 27, 39, 52], "setup-zero": [15, 165],
      }[block.id] || block.headers.map(() => width / block.headers.length);
      autoTable(doc, {
        startY: y, head: zero ? [] : [block.headers], body: block.rows,
        margin: { left, right: left, top, bottom: 20 }, tableWidth: width,
        styles: { font: "DejaVuSans", fontStyle: "normal", fontSize: zero ? 10 : 9, cellPadding: zero ? 0 : index >= report.appendixStart ? 1 : 1.5, overflow: "linebreak", textColor: 20, lineColor: 210, lineWidth: zero ? 0 : 0.1 },
        headStyles: { font: "DejaVuSans", fontStyle: "bold", fillColor: [235, 238, 240], textColor: 20 },
        alternateRowStyles: zero ? {} : { fillColor: [248, 249, 250] },
        columnStyles: widths ? Object.fromEntries(widths.map((w, i) => [i, { cellWidth: w }])) : {},
        didParseCell: ({ cell, column }) => {
          doc.setFont("DejaVuSans", cell.styles.fontStyle); doc.setFontSize(cell.styles.fontSize);
          const padding = cell.padding("left") + cell.padding("right");
          cell.text = cell.text.map((line) => readableBreaks(line, widths[column.index] - padding, (word) => doc.getTextWidth(word)));
        },
        rowPageBreak: "avoid", pageBreak: zero ? "avoid" : "auto", showHead: zero ? "never" : "everyPage", theme: zero ? "plain" : "grid",
      });
      y = doc.lastAutoTable.finalY + (zero ? 2 : 4);
    } else throw new Error(`Unknown report block: ${block.type}`);
  }
  const pages = doc.getNumberOfPages();
  for (let page = 1; page <= pages; page++) {
    doc.setPage(page);
    doc.setFont("DejaVuSans", "normal"); doc.setFontSize(9); doc.setTextColor(70);
    const title = doc.splitTextToSize(report.title, width)[0];
    doc.text(title, left, 18);
    doc.setDrawColor(180); doc.line(left, 21, 195, 21);
    doc.text(`Page ${page} of ${pages}`, 195, 281, { align: "right" });
  }
  return doc;
}

export async function downloadReportPdf(report, filename) {
  const doc = renderReportPdf(report, await loadDependencies());
  // Generate before saving/counting, so a failed font or PDF never counts success.
  const blob = doc.output("blob");
  const url = URL.createObjectURL(blob), link = document.createElement("a");
  try {
    link.href = url; link.download = filename; link.click();
  } finally { setTimeout(() => URL.revokeObjectURL(url), 1000); }
}
