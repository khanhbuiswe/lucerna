#!/usr/bin/env bash
set -euo pipefail

NAME="${1:-world}"
for i in 1 2 3; do
  echo "Hello, $NAME ($i)"
done
