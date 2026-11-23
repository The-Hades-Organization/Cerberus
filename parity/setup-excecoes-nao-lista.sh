#!/bin/sh
# excecoes.json legível que não é lista.
set -eu
d="$HOME/work/_maquina/cerberus"
mkdir -p "$d"
printf '{"a": 1}\n' >"$d/excecoes.json"
