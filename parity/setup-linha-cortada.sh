#!/bin/sh
# Histórico rotacionado (fixture `rotacao`) com a última linha cortada no meio, como depois de uma queda na escrita.
set -eu
. "$(dirname "$0")/setup-estado.sh"
printf '%s' '{"ts": "cortada' >>"$HOME/work/_maquina/cerberus/vereditos.jsonl"
