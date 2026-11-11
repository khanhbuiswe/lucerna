#!/bin/sh
# Reads one framed message, writes it back framed, then fails with "boom".
read -r header
len=$(printf %s "$header" | tr -dc '0-9')
read -r blank
body=$(head -c "$len")
printf 'Content-Length: %s\r\n\r\n%s' "$len" "$body"
echo boom >&2
exit 3
