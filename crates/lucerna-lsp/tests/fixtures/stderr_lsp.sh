#!/bin/sh
# Writes invalid UTF-8 to stderr, then more than a pipe buffer of lines.
printf 'bad \377\376 bytes\n' >&2
i=0
while [ $i -lt 3000 ]; do
  echo "line $i padding padding padding padding padding padding padding" >&2
  i=$((i + 1))
done
echo last >&2
exit 0
