#!/bin/sh
# Põe o estado pré-gravado da fixture (`pre/`: histórico e exceções gerados pelo corpo do TS com instante fixo) na
# pasta de estado do Cerberus na raiz de trabalho do caso.
set -eu
d="$HOME/work/_maquina/cerberus"
mkdir -p "$d"
if [ -d "$HOME/pre" ]; then
  mv "$HOME"/pre/* "$d"/
  rmdir "$HOME/pre"
fi
