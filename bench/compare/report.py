"""Comparison tables (Markdown) and bar charts (SVG) from a run.py result file.

    python3 -I bench/compare/report.py <result.json> --out <dir>

Writes <dir>/results.md and <dir>/<metric>-<config>.svg.

Rounding: ratios use round-half-up on the shortest decimal repr of the float
(Decimal ROUND_HALF_UP); other numbers use Python's format rounding.
Number format: Vietnamese decimal comma. Precision per unit: ms -> 0 decimals
when >= 100, else 1; MB -> 0 decimals; % -> 1 decimal; ratios -> 1 decimal.
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
# metric -> (Vietnamese label, unit), in fixed display order
METRICS = {
    "startup_ms": ("Khởi động tới khung đầu có nội dung", "ms"),
    "idle_ram_mb": ("RAM khi rỗi (repo vừa, 10 tab)", "MB"),
    "idle_cpu_pct": ("CPU khi rỗi", "%"),
    "scroll_dropped_pct": ("Khung rớt khi cuộn big.rs", "%"),
    "file_change_ms": ("Đổi file trên đĩa → màn hình cập nhật", "ms"),
    "quick_open_ms": ("Quick Open mỗi phím gõ (80.000 file)", "ms"),
    "app_size_mb": ("Dung lượng app", "MB"),
}
DASH = "—"
BAR_COLOR = "#8a8f98"


def _comma(text: str) -> str:
    return text.replace(".", ",")


def _num(v: float, unit: str) -> str:
    if unit == "ms":
        return _comma(f"{v:.0f}" if v >= 100 else f"{v:.1f}")
    if unit == "MB":
        return f"{v:.0f}"
    return _comma(f"{v:.1f}")


def fmt(v: float, unit: str) -> str:
    return f"{_num(v, unit)} {unit}"


def ratio(other, lucerna) -> str:
    if other is None or lucerna is None or lucerna == 0:
        return DASH
    q = Decimal(repr(other / lucerna)).quantize(Decimal("0.1"), rounding=ROUND_HALF_UP)
    return _comma(f"{q}") + "×"


def cell(values, unit: str, single: bool = False) -> str:
    s = run.stats(values)
    failed = f" · {s['failed']}/{len(values)} lượt hỏng" if s["failed"] else ""
    if s["median"] is None:
        return DASH + failed
    if single:
        return fmt(s["median"], unit) + failed
    return f"{fmt(s['median'], unit)} (p90 {_num(s['p90'], unit)}){failed}"


def config_label(cfg: dict) -> str:
    lsp = "rust-analyzer" if cfg.get("lsp") else "Không LSP"
    return f"{lsp} · {'lạnh' if cfg.get('cold') else 'ấm'}"


def config_slug(cfg: dict) -> str:
    return f"{'lsp' if cfg.get('lsp') else 'nolsp'}-{'cold' if cfg.get('cold') else 'warm'}"


def _medians(cfg: dict, metric: str) -> dict:
    data = cfg["metrics"][metric]
    return {e: run.stats(data.get(e, []))["median"] for e in EDITOR_ORDER}


def ratio_cells(cfg: dict, metric: str) -> list[str]:
    m = _medians(cfg, metric)
    return [DASH if e == "lucerna" else ratio(m[e], m["lucerna"]) for e in EDITOR_ORDER]


def _meta_block(meta: dict) -> list[str]:
    v = meta.get("versions", {})
    lines = [f"- Ngày: {meta.get('date', DASH)}",
             f"- Lucerna commit: {meta.get('lucerna_commit', DASH)}",
             "- Phiên bản: " + ", ".join(
                 f"{EDITOR_NAMES[e]} {v.get(e, DASH)}" for e in EDITOR_ORDER),
             f"- macOS: {meta.get('macos', DASH)}",
             f"- Chip: {meta.get('chip', DASH)}",
             f"- RAM: {meta.get('ram_gb', DASH)} GB",
             f"- Số lượt mỗi phép đo: {meta.get('rounds', DASH)}"]
    ra = meta.get("rust_analyzer")
    if ra:
        lines.append(f"- rust-analyzer: {ra.get('version') or ra.get('path') or DASH}")
    return lines


def render_markdown(result: dict) -> str:
    out = []
    if "preflight_ignored" in result.get("meta", {}):
        out += ["**Kết quả chạy thử: máy không yên khi đo (preflight bị bỏ qua), "
                "không dùng để công bố.**", ""]
    out += ["# Kết quả so sánh", ""]
    out += _meta_block(result.get("meta", {}))
    out += ["", "Mỗi ô: trung vị (p90). Dòng hệ số: trung vị của editor đó chia cho "
            "trung vị của Lucerna; giá trị lớn hơn 1 nghĩa là số đo lớn hơn số đo của Lucerna.", ""]
    for cfg in result["configs"]:
        out += [f"## {config_label(cfg)}", "",
                "| Chỉ số | Lucerna | Zed | VS Code |", "|---|---|---|---|"]
        for metric, (label, unit) in METRICS.items():
            if metric not in cfg["metrics"]:
                continue
            data = cfg["metrics"][metric]
            cells = [cell(data.get(e, []), unit, single=(metric == "app_size_mb"))
                     for e in EDITOR_ORDER]
            out.append(f"| {label} | " + " | ".join(cells) + " |")
            out.append("| ↳ so với Lucerna | " + " | ".join(ratio_cells(cfg, metric)) + " |")
        out += ["", "Biểu đồ:", ""]
        for metric, (label, _unit) in METRICS.items():
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
        value = DASH if v is None else fmt(v, unit)
        parts.append(f'<text x="{x + bar_w / 2:.1f}" y="{base - bh - 8:.1f}" '
                     f'text-anchor="middle" font-size="12">{escape(value)}</text>')
        parts.append(f'<text x="{x + bar_w / 2:.1f}" y="{base + 20}" '
                     f'text-anchor="middle" font-size="12">{escape(EDITOR_NAMES[e])}</text>')
    parts.append("</svg>")
    return "\n".join(parts) + "\n"


def write_report(result: dict, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "results.md").write_text(render_markdown(result), encoding="utf-8")
    for cfg in result["configs"]:
        for metric in METRICS:
            if metric in cfg["metrics"]:
                (out_dir / f"{metric}-{config_slug(cfg)}.svg").write_text(
                    render_svg(metric, cfg), encoding="utf-8")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("result")
    ap.add_argument("--out", required=True)
    a = ap.parse_args(sys.argv[1:] if argv is None else argv)
    result = json.loads(Path(a.result).read_text(encoding="utf-8"))
    write_report(result, Path(a.out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
