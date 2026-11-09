// Loaded only by the setup-sheet click. The report model owns all content.
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
    const lines = doc.splitTextToSize(String(text), width - indent);
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
    else if (block.type === "table") {
      // Give prose most of the width; narrow columns hold only cutting data.
      const widths = {
        operations: [6, 57, 19, 17, 20, 20, 22, 19],
        decisions: [39.6, 95.4, 45], rules: [36, 108, 36], features: [36, 63, 81],
        tools: [30, 34, 25, 37, 25, 29], header: [39, 23, 27, 39, 52],
      }[block.id];
      autoTable(doc, {
        startY: y, head: [block.headers], body: block.rows,
        margin: { left, right: left, top, bottom: 20 }, tableWidth: width,
        styles: { font: "DejaVuSans", fontStyle: "normal", fontSize: 9, cellPadding: index >= report.appendixStart ? 1 : 1.5, overflow: "linebreak", textColor: 20, lineColor: 210, lineWidth: 0.1 },
        headStyles: { font: "DejaVuSans", fontStyle: "bold", fillColor: [235, 238, 240], textColor: 20 },
        alternateRowStyles: { fillColor: [248, 249, 250] },
        columnStyles: widths ? Object.fromEntries(widths.map((w, i) => [i, { cellWidth: w }])) : {},
        rowPageBreak: "avoid", showHead: "everyPage", theme: "grid",
      });
      y = doc.lastAutoTable.finalY + 4;
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
