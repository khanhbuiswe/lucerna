"""Comparison tables (Markdown) and bar charts (SVG) from a run.py result file.

    python3 -I bench/compare/report.py <result.json> --out <dir> [--metrics a,b,...]

Writes <dir>/results.md and <dir>/<metric>-<config>.svg. --metrics limits the
tables and charts to the given metric keys (default: all). Metric keys this
report does not know, such as a dropped metric in an older result file, are
skipped.

Rounding: ratios use round-half-up on the shortest decimal repr of the float
(Decimal ROUND_HALF_UP); other numbers use Python's format rounding.
Number format: English (decimal point). Precision per unit: ms -> 0 decimals
when >= 100, else 1; MB -> 0 decimals; % -> 1 decimal (idle CPU: 2);
ratios -> 1 decimal. A ratio is not shown ("—", with a footnote) when
Lucerna's median is under the metric's noise floor (RATIO_FLOOR): a ratio of
two values that small is noise.
Ratio = other editor's median / Lucerna's median for every metric; a value
above 1 means a larger number than Lucerna's, nothing more. The output is
neutral: no winner/loser wording or colouring.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from decimal import ROUND_HALF_UP, Decimal
from xml.sax.saxutils import escape

sys.path.insert(0, str(Path(__file__).resolve().parent))

import run  # noqa: E402

EDITOR_ORDER = ("lucerna", "zed", "vscode")
EDITOR_NAMES = {"lucerna": "Lucerna", "zed": "Zed", "vscode": "VS Code"}
# metric -> (label, unit), in fixed display order
METRICS = {
    "startup_ms": ("Startup to first frame with content", "ms"),
    "idle_ram_mb": ("Idle RAM (medium repo, 10 tabs)", "MB"),
    "idle_cpu_pct": ("Idle CPU", "%"),
    "scroll_dropped_pct": ("Dropped frames scrolling big.rs", "%"),
    "file_change_ms": ("File changed on disk → screen updated", "ms"),
    "app_size_mb": ("App size", "MB"),
}
DASH = "—"
BAR_COLOR = "#8a8f98"
DECIMALS = {"idle_cpu_pct": 2}   # per-metric override of the unit precision
CPU_RATIO_FLOOR = 0.1            # %: idle CPU below this is measurement noise
RATIO_FLOOR = {"idle_cpu_pct": CPU_RATIO_FLOOR}
FLOOR_NOTE = {"idle_cpu_pct": "Ratio not computed when Lucerna's value is below the "
                              "measurable threshold (CPU < 0.1 %)."}


def _num(v: float, unit: str, decimals: int | None = None) -> str:
    if decimals is not None:
        return f"{v:.{decimals}f}"
    if unit == "ms":
        return f"{v:.0f}" if v >= 100 else f"{v:.1f}"
    if unit == "MB":
        return f"{v:.0f}"
    return f"{v:.1f}"


def fmt(v: float, unit: str, decimals: int | None = None) -> str:
    return f"{_num(v, unit, decimals)} {unit}"


def ratio(other, lucerna) -> str:
    if other is None or lucerna is None or lucerna == 0:
        return DASH
    q = Decimal(repr(other / lucerna)).quantize(Decimal("0.1"), rounding=ROUND_HALF_UP)
    return f"{q}×"


def cell(values, unit: str, single: bool = False, decimals: int | None = None) -> str:
    s = run.stats(values)
    failed = f" · {s['failed']}/{len(values)} rounds failed" if s["failed"] else ""
    if s["median"] is None:
        return DASH + failed
    if single:
        return fmt(s["median"], unit, decimals) + failed
    return (f"{fmt(s['median'], unit, decimals)} (p90 {_num(s['p90'], unit, decimals)})"
            f"{failed}")


def config_label(cfg: dict) -> str:
    lsp = "rust-analyzer" if cfg.get("lsp") else "No LSP"
    # --cold purges only before startup launches: the other metrics are warm.
    return f"{lsp} · {'cold (startup only)' if cfg.get('cold') else 'warm'}"


def config_slug(cfg: dict) -> str:
    return f"{'lsp' if cfg.get('lsp') else 'nolsp'}-{'cold' if cfg.get('cold') else 'warm'}"


def _medians(cfg: dict, metric: str) -> dict:
    data = cfg["metrics"][metric]
    return {e: run.stats(data.get(e, []))["median"] for e in EDITOR_ORDER}


def below_floor(cfg: dict, metric: str) -> bool:
    """Lucerna's median is under the metric's noise floor: no ratio."""
    floor = RATIO_FLOOR.get(metric)
    lucerna = _medians(cfg, metric)["lucerna"]
    return floor is not None and lucerna is not None and lucerna < floor


def ratio_cells(cfg: dict, metric: str) -> list[str]:
    m = _medians(cfg, metric)
    if below_floor(cfg, metric):
        return [DASH] * len(EDITOR_ORDER)
    return [DASH if e == "lucerna" else ratio(m[e], m["lucerna"]) for e in EDITOR_ORDER]


def _meta_block(meta: dict) -> list[str]:
    v = meta.get("versions", {})
    lines = [f"- Date: {meta.get('date', DASH)}",
             f"- Lucerna commit: {meta.get('lucerna_commit', DASH)}",
             "- Versions: " + ", ".join(
                 f"{EDITOR_NAMES[e]} {v.get(e, DASH)}" for e in EDITOR_ORDER),
             f"- macOS: {meta.get('macos', DASH)}",
             f"- Chip: {meta.get('chip', DASH)}",
             f"- RAM: {meta.get('ram_gb', DASH)} GB",
             f"- Rounds per measurement: {meta.get('rounds', DASH)}"]
    ra = meta.get("rust_analyzer")
    if ra:
        lines.append(f"- rust-analyzer: {ra.get('version') or ra.get('path') or DASH}")
    lb = meta.get("lucerna_binary")
    if lb:
        lines.append(f"- Lucerna binary: {lb.get('version') or DASH}, "
                     f"built at {lb.get('mtime') or DASH}")
    d = meta.get("display")
    if d:
        lines.append(f"- Main display: {d.get('width')}×{d.get('height')} pt, "
                     f"scale {d.get('scale')}, {d.get('refresh_hz')} Hz")
    wf = meta.get("window_frame")
    if wf:
        lines.append(f"- Window: {wf.get('w')}×{wf.get('h')} pt (all measurements except startup; "
                     "startup uses each editor's default size)")
    return lines


def _selected(metrics) -> list[str]:
    """Metric keys to report, in the fixed display order."""
    return [m for m in METRICS if metrics is None or m in metrics]


def parse_metrics(text: str) -> list[str]:
    """Validate a comma-separated --metrics value; raise ValueError on unknown names."""
    names = [n.strip() for n in text.split(",") if n.strip()]
    unknown = [n for n in names if n not in METRICS]
    if not names or unknown:
        raise ValueError(f"unknown metric(s): {', '.join(unknown) or text!r}; "
                         f"valid: {', '.join(METRICS)}")
    return names


def render_markdown(result: dict, metrics=None) -> str:
    out = []
    if "preflight_ignored" in result.get("meta", {}):
        out += ["**Trial result: the machine was not quiet during measurement (preflight "
                "ignored); do not publish.**", ""]
    out += ["# Comparison results", ""]
    out += _meta_block(result.get("meta", {}))
    out += ["", "Each cell: median (p90). Ratio row: that editor's median divided by "
            "Lucerna's median; a value above 1 means a larger measurement than Lucerna's.", ""]
    for cfg in result["configs"]:
        out += [f"## {config_label(cfg)}", "",
                "| Metric | Lucerna | Zed | VS Code |", "|---|---|---|---|"]
        notes = []
        for metric in _selected(metrics):
            label, unit = METRICS[metric]
            if metric not in cfg["metrics"]:
                continue
            data = cfg["metrics"][metric]
            cells = [cell(data.get(e, []), unit, single=(metric == "app_size_mb"),
                          decimals=DECIMALS.get(metric))
                     for e in EDITOR_ORDER]
            out.append(f"| {label} | " + " | ".join(cells) + " |")
            out.append("| ↳ vs Lucerna | " + " | ".join(ratio_cells(cfg, metric)) + " |")
            if below_floor(cfg, metric):
                notes.append(FLOOR_NOTE[metric])
        if notes:
            out += [""] + notes
        out += ["", "Charts:", ""]
        for metric in _selected(metrics):
            label = METRICS[metric][0]
            if metric in cfg["metrics"]:
                out.append(f"- [{label}]({metric}-{config_slug(cfg)}.svg)")
        out.append("")
    return "\n".join(out)


def render_svg(metric: str, cfg: dict) -> str:
    label, unit = METRICS[metric]
    med = _medians(cfg, metric)
    top = max([v for v in med.values() if v is not None] or [0]) or 1
    w, h, base, plot_h = 480, 300, 240, 160
    bar_w, gap = 90, 40
    x0 = (w - (3 * bar_w + 2 * gap)) / 2
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
             f'width="{w}" height="{h}" role="img" font-family="sans-serif">',
             '<style>text{fill:#24292f}line{stroke:#8a8f98}'
             '@media (prefers-color-scheme:dark){text{fill:#e6e8eb}}</style>',
             f'<title>{escape(label)} — {escape(config_label(cfg))}</title>',
             f'<text x="{w / 2}" y="24" text-anchor="middle" font-size="14">'
             f'{escape(label)} ({escape(config_label(cfg))})</text>',
             f'<line x1="{x0 - 20}" y1="{base}" x2="{w - x0 + 20}" y2="{base}" '
             f'stroke="#8a8f98" stroke-width="1"/>']
    for i, e in enumerate(EDITOR_ORDER):
        x = x0 + i * (bar_w + gap)
        v = med[e]
        bh = 0 if v is None else v / top * plot_h
        parts.append(f'<rect class="bar" x="{x:.1f}" y="{base - bh:.1f}" width="{bar_w}" '
                     f'height="{bh:.1f}" fill="{BAR_COLOR}"/>')
        value = DASH if v is None else fmt(v, unit, DECIMALS.get(metric))
        parts.append(f'<text x="{x + bar_w / 2:.1f}" y="{base - bh - 8:.1f}" '
                     f'text-anchor="middle" font-size="12">{escape(value)}</text>')
        parts.append(f'<text x="{x + bar_w / 2:.1f}" y="{base + 20}" '
                     f'text-anchor="middle" font-size="12">{escape(EDITOR_NAMES[e])}</text>')
    parts.append("</svg>")
    return "\n".join(parts) + "\n"


def write_report(result: dict, out_dir: Path, metrics=None) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "results.md").write_text(render_markdown(result, metrics), encoding="utf-8")
    for cfg in result["configs"]:
        for metric in _selected(metrics):
            if metric in cfg["metrics"]:
                (out_dir / f"{metric}-{config_slug(cfg)}.svg").write_text(
                    render_svg(metric, cfg), encoding="utf-8")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("result")
    ap.add_argument("--out", required=True)
    ap.add_argument("--metrics", help="comma-separated metric keys to include "
                    f"(default: all). Keys: {', '.join(METRICS)}")
    a = ap.parse_args(sys.argv[1:] if argv is None else argv)
    metrics = None
    if a.metrics is not None:
        try:
            metrics = parse_metrics(a.metrics)
        except ValueError as e:
            ap.error(str(e))
    result = json.loads(Path(a.result).read_text(encoding="utf-8"))
    write_report(result, Path(a.out), metrics)
    return 0


if __name__ == "__main__":
    sys.exit(main())
