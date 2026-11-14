#!/bin/sh
# Build bench-helper with the Command Line Tools swiftc (no Xcode needed).
# Usage: build.sh [output path]
# Default output: ${BENCH_DIR:-$HOME/LucernaBench.noindex}/bin/bench-helper
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-${BENCH_DIR:-$HOME/LucernaBench.noindex}/bin/bench-helper}
mkdir -p "$(dirname "$out")"
# -swift-version 5: the SCStream delegate callbacks arrive on our own dispatch
# queue; Swift 6 strict concurrency would demand Sendable plumbing that adds
# nothing for a single-shot CLI tool.
exec swiftc -O -swift-version 5 \
  -framework ScreenCaptureKit -framework CoreGraphics -framework AppKit \
  -framework CoreMedia -framework CoreVideo -framework ImageIO \
  "$here/main.swift" -o "$out"
