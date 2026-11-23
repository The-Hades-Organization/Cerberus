#!/usr/bin/env bash
# Bench de partida do binário: média de N execuções (cache quente) de `modos`, `registrar`, `guardas --json` (35
# guardas) e `avaliar` sem guarda `exec`, numa skill e num HOME fictícios. Sai 1 se algum caso passar do limite.
#
# uso: scripts/bench-partida.sh <binário> [limite-ms=50] [execuções=15]
set -euo pipefail

bin=$(realpath "${1:?uso: scripts/bench-partida.sh <binário> [limite-ms] [execuções]}")
limite=${2:-50}
n=${3:-15}

caixa=$(mktemp -d)
trap 'rm -rf "$caixa"' EXIT
skill=$caixa/skill
mkdir -p "$skill/kit" "$caixa/work"
printf 'HADES_CERBERUS_PORTA\tbarra\nHADES_CERBERUS_LIMIAR\tbarra\nHADES_CERBERUS_VIGILIA\tavisa\nHADES_CERBERUS_TIMEOUT_S\t60\nHADES_CERBERUS_HIST_KB\t2048\nHADES_CERBERUS_EXC_TTL_H\t24\nHADES_JOURNAL\t0\n' >"$skill/kit/config.tsv"
for i in $(seq 1 35); do
  case $((i % 3)) in 0) cab=porta ;; 1) cab=limiar ;; *) cab=vigilia ;; esac
  mkdir -p "$skill/loop/cerberus/$cab"
  printf '{"titulo": "Guarda %s", "origem": "bench", "momentos": ["release", "done"], "tipo": "emitida"}\n' "$i" \
    >"$skill/loop/cerberus/$cab/g$i.json"
done

export HOME=$caixa HADES_HOME=$caixa/hh HADES_WORK_ROOT=$caixa/work HADES_SKILL_DIR=$skill TZ=UTC LANG=C.UTF-8 NO_COLOR=1
cd "$caixa"

# média em ms (uma casa) de N execuções, depois de uma de aquecimento que tem de sair 0 ou 1 (barrou)
medir() {
  "$bin" "$@" >/dev/null 2>&1 || { [ $? -le 1 ] || { echo "falhou: $*" >&2; exit 2; }; }
  local ini fim
  ini=$(date +%s%N)
  for _ in $(seq 1 "$n"); do "$bin" "$@" >/dev/null 2>&1 || true; done
  fim=$(date +%s%N)
  echo $(((fim - ini) / n / 100000)) | sed -E 's/([0-9])$/.\1/; s/^\./0./'
}

falhou=0
printf '%-16s %8s  %s\n' caso média limite
while IFS='|' read -r nome meta args; do
  # shellcheck disable=SC2086
  ms=$(medir $args)
  estado=ok
  if awk -v m="$ms" -v l="$limite" 'BEGIN { exit !(m > l) }'; then
    estado=ACIMA
    falhou=1
  fi
  printf '%-16s %6s ms  meta %s ms, falha > %s ms: %s\n' "$nome" "$ms" "$meta" "$limite" "$estado"
done <<EOF
modos|10|modos
registrar|10|registrar --cabeca porta --guarda g3 --momento release --resultado ok --evidencia bench|bench
guardas --json|20|guardas --json
avaliar|20|avaliar release
EOF
exit "$falhou"
