#!/usr/bin/env bash
# Stub do kit/skill-release.sh da skill: a guarda nome-dominio chama `skill-release.sh guard`.
# A resposta vem de $PARITY_FIX/skill-release/<subcomando>.{out,rc}; sem arquivo, passa calado.
fix="${PARITY_FIX:-$HOME/fix}/skill-release"
sub="${1:-}"
[ -f "$fix/$sub.out" ] && cat "$fix/$sub.out"
[ -f "$fix/$sub.err" ] && cat "$fix/$sub.err" >&2
exit "$(cat "$fix/$sub.rc" 2>/dev/null || echo 0)"
