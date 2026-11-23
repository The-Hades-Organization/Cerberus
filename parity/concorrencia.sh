#!/usr/bin/env bash
# Concorrência TS × Rust sobre a mesma raiz de trabalho (plano F4, Cerberus §2.1): 8 `avaliar push` em paralelo, 4 pelo
# TS e 4 pelo binário Rust, com uma exceção de uso único aberta. Os dois lados gravam sob o mesmo flock(2) de
# `<store>/.lock`; se um deles não respeitar o lock, linhas se perdem ou se misturam, ou a exceção é gasta duas vezes.
#   concorrencia.sh <skill montada> <binário cerberus>
# Confere: o histórico tem 8×N linhas, todas JSON válido (N = vereditos de um `avaliar push` sozinho); exatamente um
# veredito usou a exceção e ela ficou marcada como usada uma vez; exatamente uma avaliação passou (exit 0) e as outras
# sete barraram (exit 1).
set -euo pipefail
skill=$(cd "$1" && pwd)
rust=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

ambiente() {
  export HOME=$1 HADES_WORK_ROOT=$1/work HADES_SKILL_DIR=$skill TZ=UTC LANG=C.UTF-8 NO_COLOR=1
  export HADES_CERBERUS_PORTA=barra HADES_CERBERUS_TIMEOUT_S=3
  export PATH="$skill/loop/test/stubs:$PATH"
}
store() { echo "$1/work/_maquina/cerberus"; }

# N: vereditos de um `avaliar push` sozinho, numa raiz à parte
(
  ambiente "$T/solo"
  mkdir -p "$HOME"
  cd "$HOME"
  bun "$skill/src/cli.ts" cerberus avaliar push >/dev/null 2>&1 || true
)
n=$(wc -l <"$(store "$T/solo")/vereditos.jsonl")
[ "$n" -gt 0 ] || { echo "concorrência: avaliar push sozinho não gravou veredito" >&2; exit 1; }

# a raiz comum: exceções pré-gravadas; só a de alvo pr=9 serve, e é de uso único
H=$T/comum
mkdir -p "$(store "$H")"
cp "$here/fixtures/excecoes/pre/excecoes.json" "$(store "$H")/"

pids=()
for i in 1 2 3 4 5 6 7 8; do
  (
    ambiente "$H"
    cd "$H"
    rc=0
    if [ $((i % 2)) -eq 0 ]; then
      "$rust" avaliar push --ctx pr=9 >"$T/out-$i" 2>&1 || rc=$?
    else
      bun "$skill/src/cli.ts" cerberus avaliar push --ctx pr=9 >"$T/out-$i" 2>&1 || rc=$?
    fi
    echo "$rc" >"$T/rc-$i"
  ) &
  pids+=($!)
done
for p in "${pids[@]}"; do wait "$p"; done

python3 - "$(store "$H")" "$n" "$T" <<'EOF'
import json, sys
from collections import Counter
store, n, tmp = sys.argv[1], int(sys.argv[2]), sys.argv[3]
erros = []
linhas = open(f"{store}/vereditos.jsonl", encoding="utf-8").read().splitlines()
if len(linhas) != 8 * n:
    erros.append(f"histórico com {len(linhas)} linhas; esperava 8×{n} = {8 * n}")
regs = []
for i, l in enumerate(linhas, 1):
    try:
        regs.append(json.loads(l))
    except ValueError:
        erros.append(f"linha {i} do histórico não é JSON: {l[:80]!r}")
usaram = [r for r in regs if "excecao" in r]
if len(usaram) != 1:
    erros.append(f"{len(usaram)} vereditos usaram a exceção; esperava 1")
excs = json.load(open(f"{store}/excecoes.json", encoding="utf-8"))
alvo = [e for e in excs if e.get("alvo") == {"pr": "9"}]
if len(alvo) != 1 or not alvo[0].get("usada"):
    erros.append(f"exceção de alvo pr=9 não ficou usada uma vez: {alvo}")
elif usaram and alvo[0]["usada"] != usaram[0]["avaliacao"]:
    erros.append("a exceção aponta outra avaliação que não a que a usou")
rcs = Counter(open(f"{tmp}/rc-{i}").read().strip() for i in range(1, 9))
if rcs != Counter({"0": 1, "1": 7}):
    erros.append(f"códigos de saída {dict(rcs)}; esperava um 0 e sete 1")
for e in erros:
    print("concorrência:", e, file=sys.stderr)
if erros:
    sys.exit(1)
print(f"concorrência: ok (8 avaliações, {8 * n} vereditos, exceção gasta uma vez)")
EOF
