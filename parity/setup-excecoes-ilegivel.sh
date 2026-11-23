#!/bin/sh
# excecoes.json que não é JSON.
set -eu
d="$HOME/work/_maquina/cerberus"
mkdir -p "$d"
printf '{\n' >"$d/excecoes.json"
