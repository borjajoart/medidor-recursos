// Gráficas SVG mínimas sin dependencias. Las usan el informe y la comparación.
// chart(hostId, {unit, series:[{n, c, pts:[[t,v],...], dash}], min, max, auto,
//                lines:[{v,n,c}], bands:[{a,b,c}], marks:[{t,label}], tip:(t)=>html, hide})
const NS = "http://www.w3.org/2000/svg";
const CH = { W: 1000, H: 210, ML: 50, MR: 12, MT: 14, MB: 24 };
const allCharts = [];

function el(tag, attrs, parent) {
  const e = document.createElementNS(NS, tag);
  for (const k in attrs) e.setAttribute(k, attrs[k]);
  if (parent) parent.appendChild(e);
  return e;
}
function niceStep(v) {
  if (v <= 0) return 1;
  const p = Math.pow(10, Math.floor(Math.log10(v)));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= v) return m * p;
  return 10 * p;
}
function esc(s) {
  return String(s).replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}
function clock(t, t0h) {
  // t0h: "HH:MM:SS" del inicio; devuelve la hora real de t segundos después
  if (!t0h) { t = Math.round(t); return Math.floor(t / 60) + ":" + String(t % 60).padStart(2, "0"); }
  const [h, m, s] = t0h.split(":").map(Number);
  let x = Math.round(h * 3600 + m * 60 + s + t) % 86400;
  return [Math.floor(x / 3600), Math.floor(x / 60) % 60, x % 60].map(v => String(v).padStart(2, "0")).join(":");
}
function nearest(pts, t) {
  let lo = 0, hi = pts.length - 1;
  if (hi < 0) return -1;
  while (lo < hi) { const m = (lo + hi) >> 1; if (pts[m][0] < t) lo = m + 1; else hi = m; }
  if (lo > 0 && Math.abs(pts[lo - 1][0] - t) < Math.abs(pts[lo][0] - t)) lo--;
  return lo;
}

function chart(id, o) {
  const host = document.getElementById(id);
  const series = o.series.filter(s => s.pts.length);
  if (!series.length) {
    if (o.hide) document.getElementById(o.hide).style.display = "none";
    else host.innerHTML = '<p class="muted">Sin datos para esta gráfica.</p>';
    return;
  }
  const { W, H, ML, MR, MT, MB } = CH;
  let t0 = Infinity, t1 = -Infinity, vmax = -Infinity, vmin = Infinity;
  for (const s of series) for (const [t, v] of s.pts) {
    t0 = Math.min(t0, t); t1 = Math.max(t1, t); vmax = Math.max(vmax, v); vmin = Math.min(vmin, v);
  }
  if (o.t0 != null) t0 = o.t0;
  if (o.t1 != null) t1 = o.t1;
  if (t1 <= t0) t1 = t0 + 1;
  (o.lines || []).forEach(l => vmax = Math.max(vmax, l.v));
  let ymin = o.auto ? Math.max(0, vmin - 5) : (o.min || 0);
  let ymax = o.max || Math.max(vmax * 1.05, ymin + 1);
  const step = o.max ? (ymax - ymin) / 4 : niceStep((ymax - ymin) / 4);
  ymin = Math.floor(ymin / step) * step;
  if (!o.max) ymax = Math.ceil(ymax / step) * step;
  const X = t => ML + (t - t0) / (t1 - t0) * (W - ML - MR);
  const Y = v => MT + (1 - (Math.min(Math.max(v, ymin), ymax) - ymin) / (ymax - ymin)) * (H - MT - MB);

  const lg = document.createElement("div");
  lg.className = "legend";
  lg.innerHTML = series.map(s => `<span><i style="background:${s.c}${s.dash ? ";opacity:.6" : ""}"></i>${esc(s.n)}</span>`).join("");
  host.appendChild(lg);
  const wrap = document.createElement("div");
  wrap.className = "chart";
  host.appendChild(wrap);
  const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, role: "img" }, wrap);

  for (const b of o.bands || []) {
    if (b.b < t0 || b.a > t1) continue;
    el("rect", { x: X(b.a), y: MT, width: Math.max(2, X(b.b) - X(b.a)), height: H - MT - MB, fill: b.c, opacity: .16 }, svg);
  }
  for (let v = ymin; v <= ymax + 1e-9; v += step) {
    const y = Y(v);
    el("line", { x1: ML, x2: W - MR, y1: y, y2: y, class: "grid" }, svg);
    el("text", { x: ML - 6, y: y + 4, "text-anchor": "end", class: "ax" }, svg).textContent = (Math.round(v * 10) / 10) + o.unit;
  }
  for (let i = 0; i <= 6; i++) {
    const t = t0 + (t1 - t0) * i / 6;
    el("text", { x: X(t), y: H - 6, "text-anchor": i == 0 ? "start" : i == 6 ? "end" : "middle", class: "ax" }, svg).textContent = clock(t, o.clock);
  }
  for (const l of o.lines || []) {
    const y = Y(l.v);
    el("line", { x1: ML, x2: W - MR, y1: y, y2: y, stroke: l.c, "stroke-dasharray": "6 4", "stroke-width": 1, opacity: .7 }, svg);
    el("text", { x: W - MR - 4, y: y - 4, "text-anchor": "end", class: "ax" }, svg).textContent = l.n;
  }
  for (const m of o.marks || []) {
    if (m.t < t0 || m.t > t1) continue;
    const x = X(m.t);
    el("line", { x1: x, x2: x, y1: MT, y2: H - MB, class: "mark" }, svg);
    el("text", { x: x + 3, y: MT + 9, class: "markl" }, svg).textContent = m.label;
  }
  for (const s of series) {
    let p = "", pen = false, last = null;
    const gap = o.gap || Infinity;
    for (const [t, v] of s.pts) {
      if (last != null && t - last > gap) pen = false;
      p += (pen ? "L" : "M") + X(t).toFixed(1) + " " + Y(v).toFixed(1);
      pen = true; last = t;
    }
    el("path", { d: p, fill: "none", stroke: s.c, "stroke-width": s.w || 1.5, "stroke-linejoin": "round", "stroke-dasharray": s.dash ? "5 3" : "none" }, svg);
  }
  const cur = el("line", { y1: MT, y2: H - MB, class: "cursor", opacity: 0 }, svg);
  const dots = series.map(s => el("circle", { r: 3, fill: s.c, opacity: 0 }, svg));
  const tip = document.createElement("div");
  tip.className = "tip";
  wrap.appendChild(tip);
  const ch = { X, Y, t0, t1, cur, dots, series, tip, o };
  allCharts.push(ch);
  el("rect", { x: ML, y: MT, width: W - ML - MR, height: H - MT - MB, fill: "transparent" }, svg);
  svg.addEventListener("mousemove", e => {
    const r = svg.getBoundingClientRect();
    const t = t0 + ((e.clientX - r.left) / r.width * W - ML) / (W - ML - MR) * (t1 - t0);
    showAt(Math.min(Math.max(t, t0), t1), ch, e.clientX - r.left, e.clientY - r.top, r.width);
  });
  svg.addEventListener("mouseleave", () => allCharts.forEach(c => {
    c.cur.setAttribute("opacity", 0); c.dots.forEach(d => d.setAttribute("opacity", 0)); c.tip.style.display = "none";
  }));
}

function showAt(t, src, mx, my, w) {
  for (const c of allCharts) {
    if (t < c.t0 || t > c.t1) { c.cur.setAttribute("opacity", 0); continue; }
    const x = c.X(t);
    c.cur.setAttribute("x1", x); c.cur.setAttribute("x2", x); c.cur.setAttribute("opacity", 1);
    c.series.forEach((s, k) => {
      const i = nearest(s.pts, t), d = c.dots[k];
      if (i < 0 || Math.abs(s.pts[i][0] - t) > (c.o.snap || 6)) { d.setAttribute("opacity", 0); return; }
      d.setAttribute("cx", c.X(s.pts[i][0])); d.setAttribute("cy", c.Y(s.pts[i][1])); d.setAttribute("opacity", 1);
    });
    if (c !== src) c.tip.style.display = "none";
  }
  let h = `<div class="th">${clock(t, src.o.clock)}</div>`;
  for (const s of src.series) {
    const i = nearest(s.pts, t);
    if (i >= 0 && Math.abs(s.pts[i][0] - t) <= (src.o.snap || 6)) {
      const v = s.pts[i][1];
      h += `<div class="row"><span><i style="background:${s.c}"></i>${esc(s.n)}</span><b>${v.toFixed(v < 10 ? 1 : 0)}${src.o.unit}</b></div>`;
    }
  }
  if (src.o.tip) h += src.o.tip(t);
  const tp = src.tip;
  tp.innerHTML = h;
  tp.style.display = "block";
  const tw = tp.offsetWidth;
  tp.style.left = (mx + 14 + tw > w ? mx - tw - 14 : mx + 14) + "px";
  tp.style.top = Math.max(0, my - 30) + "px";
}
