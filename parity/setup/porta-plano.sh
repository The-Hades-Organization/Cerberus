. "$(dirname "$0")/porta-lib.sh"
SHA=6e2d7c784276faf6ce2ccdb900ef92f3b104c2398ff1f4753f3ac7d6925a1f5e
repo r
printf '# Plano\n\n## Objetivo\nx\n\n## Estado atual\nv1\n' >plan.md
printf '# Plano\n\n## Objetivo\nx\n\n## Estado atual (27/10)\nv2\n' >plan-estado.md
printf '# Plano\n\n## Objetivo\ny\n' >plan-mudou.md
reg() { printf '%s' "$2" >"$1"; }
reg plan-ok.json "{\"path\":\"$HOME/plan.md\",\"sha\":\"$SHA\",\"approvedAt\":\"2026-01-01T00:00:00Z\",\"by\":{\"user\":\"dono\",\"origem\":\"painel\"}}"
reg plan-sem-by.json "{\"path\":\"$HOME/plan.md\",\"sha\":\"$SHA\"}"
reg plan-by-texto.json "{\"path\":\"$HOME/plan.md\",\"sha\":\"$SHA\",\"by\":\"dono\"}"
reg plan-estado.json "{\"path\":\"$HOME/plan-estado.md\",\"sha\":\"$SHA\"}"
reg plan-mudou.json "{\"path\":\"$HOME/plan-mudou.md\",\"sha\":\"$SHA\"}"
reg plan-sumiu.json "{\"path\":\"$HOME/sumiu.md\",\"sha\":\"$SHA\"}"
reg plan-ilegivel.json '{não é json'
reg plan-sem-sha.json "{\"path\":\"$HOME/plan.md\"}"
