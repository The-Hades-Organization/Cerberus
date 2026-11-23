#!/bin/sh
# Roda o comando e troca, em stdout e stderr, o texto do erro do parser de JSON (do Bun no TS, do serde no Rust) por
# `<erro do parser>`, como o golden legado faz (divergência D5); preserva o código de saída.
#   normalizar-parser.sh <programa> [args...]
t=$(mktemp -d)
"$@" >"$t/out" 2>"$t/err"
rc=$?
n() { sed -E -e 's/(ilegível|ilegível ou sem path\/sha): [^"]*/\1: <erro do parser>/' -e 's/ilegível \((JSON Parse error: [^)]*|[^()"]* at line [0-9]+ column [0-9]+)\)/ilegível (<erro do parser>)/g' "$1"; }
n "$t/out"
n "$t/err" >&2
rm -rf "$t"
exit "$rc"
