# Comparison results

- Date: 2026-11-15
- Lucerna commit: 026bbac
- Versions: Lucerna luc 0.1.0 (build 2026-11-15T09:37:21+00:00), Zed 1.28.1, VS Code 1.146.0
- macOS: 26.7.1 (25G241)
- Chip: Apple M4
- RAM: 16 GB
- Rounds per measurement: 10
- Lucerna binary: luc 0.1.0, built at 2026-11-15T09:37:21+00:00
- Main display: 1470×956 pt, scale 2, 60 Hz
- Window: 1280×800 pt (all measurements except startup; startup uses each editor's default size)

Each cell: median (p90). Ratio row: that editor's median divided by Lucerna's median; a value above 1 means a larger measurement than Lucerna's.

## No LSP · warm

| Metric | Lucerna | Zed | VS Code |
|---|---|---|---|
| Startup to first frame with content | 140 ms (p90 143) | 241 ms (p90 252) | 267 ms (p90 672) |
| ↳ vs Lucerna | — | 1.7× | 1.9× |
| Idle RAM (medium repo, 10 tabs) | 169 MB (p90 179) | 235 MB (p90 236) | 879 MB (p90 879) |
| ↳ vs Lucerna | — | 1.4× | 5.2× |
| Idle CPU | 1.40 % (p90 1.42) | 2.20 % (p90 2.23) | 3.01 % (p90 3.18) |
| ↳ vs Lucerna | — | 1.6× | 2.1× |
| App size | 50 MB | 177 MB | 1604 MB |
| ↳ vs Lucerna | — | 3.5× | 31.9× |

Charts:

- [Startup to first frame with content](startup_ms-nolsp-warm.svg)
- [Idle RAM (medium repo, 10 tabs)](idle_ram_mb-nolsp-warm.svg)
- [Idle CPU](idle_cpu_pct-nolsp-warm.svg)
- [App size](app_size_mb-nolsp-warm.svg)
