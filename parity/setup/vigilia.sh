# Setup da matriz da Vigília (cases-vigilia.toml e os casos vigilia.* de cases-parser.toml). Molde:
# loop/test/ts/cerberus-vigilia-legado.test.ts do Hades-Skill, que monta tudo numa raiz só e acumula o estado; aqui cada
# caso monta só o que o veredito dele lê. O caso vem do `cwd` (`vNNN`, o número da chamada no golden legado), que o
# runner cria no HOME antes do setup: essa pasta vira o repositório do caso (a worktree do agente principal).
# Repositório acme/widget; registro do orquestrador em work/acme__widget/orchestration; `gh pr list` e
# `herdr agent list` respondem de $HOME/fix (parity/stubs), no lugar do STUB_PRS/STUB_HERDR do checks-rig.ts.
. "$(dirname "$0")/porta-lib.sh"

caso=$(ls -d v[0-9][0-9][0-9] 2>/dev/null | head -1)
[ -n "$caso" ] || { echo "vigilia.sh: sem pasta vNNN no HOME (cwd do caso)" >&2; exit 1; }
n=${caso#v}
W=$HOME/work/acme__widget
ORCH=$W/orchestration
JOURNAL=$W/locks/gate-journal.tsv
mkdir -p "$ORCH/agents" "$ORCH/consolidations" "$W/locks"

# agente <nome> <worktree>: repositório novo em <worktree>, .design-sync-reports com report.md e registro do agente.
# Opcionais, lidos uma vez e zerados: REPORT (texto com \n; SEM_REPORT=1 para ausente), BRIEF, QUESTIONS e os campos
# do registro BRANCH, BASE (padrão: HEAD), MODE, STATE, PR, PANE, PORT, CREATED, WT (worktree gravada no registro).
agente() {
  nome=$1; wt=$2
  [ -d "$wt/.git" ] || repo "$wt"
  mkdir -p "$wt/.design-sync-reports"
  [ -n "${SEM_REPORT-}" ] || printf '%b' "${REPORT-STATUS: DONE\n}" >"$wt/.design-sync-reports/report.md"
  [ -z "${BRIEF+x}" ] || printf '%b' "$BRIEF" >"$wt/.design-sync-reports/brief.md"
  [ -z "${QUESTIONS+x}" ] || printf '%b' "$QUESTIONS" >"$wt/.design-sync-reports/questions.md"
  base=${BASE-$(g "$wt" rev-parse HEAD)}
  printf '{"name":"%s","worktree":"%s","branch":"%s","base":"%s","mode":"%s","state":"%s","pr":"%s","pane":"%s","port":"%s","createdAt":"%s"}' \
    "$nome" "${WT-$wt}" "${BRANCH-feature}" "$base" "${MODE-impl}" "${STATE-active}" "${PR-7}" "${PANE-w1:p1}" \
    "${PORT-21290}" "${CREATED-2000-01-01T00:00:00Z}" >"$ORCH/agents/$nome.json"
  unset REPORT SEM_REPORT BRIEF QUESTIONS BRANCH BASE MODE STATE PR PANE PORT CREATED WT
}

# o agente principal do caso: ag<NNN>, worktree = pasta do caso
principal() { agente "ag$n" "$HOME/$caso"; }

# linha do journal da fila: linha <kind> <label> <rc> [ts]
linha() { printf '%s\tdone\t%s\t%s\t1\t%s\trc=%s\n' "${4-2026-01-01T00:00:00Z}" "$1" "$2" "$HOME/$caso" "$3" >>"$JOURNAL"; }

# SHA curto do HEAD com dígito e letra (loop/test/ts/sha-citavel.ts)
sha_citavel() {
  while :; do
    s=$(g "$1" rev-parse --short HEAD)
    case $s in *[0-9]*) case $s in *[a-f]*) echo "$s"; return ;; esac ;; esac
    g "$1" commit -q --allow-empty -m outro
  done
}

# arquivos <wt> <arquivo> <texto> ...: grava e commita tudo fora de .design-sync-reports
arquivos() {
  wt=$1; shift
  while [ $# -ge 2 ]; do mkdir -p "$(dirname "$wt/$1")"; printf '%s' "$2" >"$wt/$1"; shift 2; done
  g "$wt" add -A -- . ':!.design-sync-reports'
  g "$wt" commit -qm muda
}

# branch lateral com um commit; imprime o sha completo e volta para feature
lateral() {
  g "$1" checkout -q -b "$2"
  printf '%b' "$3" >"$1/$4"
  g "$1" add "$4"
  g "$1" commit -qm "$5"
  g "$1" rev-parse HEAD
  g "$1" checkout -q feature
}

herdr_json() { resposta herdr agent-list "{\"result\":{\"agents\":[$1]}}"; }
viva() { herdr_json "{\"name\":\"ag$n\",\"pane_id\":\"${1-w1:p1}\",\"agent_status\":\"idle\"}"; }
prs() { resposta gh pr-list "$1"; }
pr7() { prs "[{\"number\":${2-7},\"state\":\"${1-OPEN}\",\"headRefName\":\"${3-feature}\"}]"; }

plano() {
  if [ "$1" = - ]; then printf '# Plano\n\n## Objetivo\nx\n' >"$HOME/plano.md"
  else printf '# Plano\n\n## Objetivo\nx\n\n## Estado atual\n<!-- modelo -->\n%b\n\n## Depois\ny\n' "$1" >"$HOME/plano.md"; fi
  printf '{"path":"%s"}' "$HOME/plano.md" >"$ORCH/plan.json"
}

BRIEF_ESCOPO='## Item\n\n**Arquivos seus (e só estes):** `src/`, `docs/*.md`, `kit/<x>.sh`\n\n**Em voo, não toque:** `lib.sh`, `loop/bin/`\n'

# consolidação: <branch sem hades/> <primeiro PR> <quantos> <report dos membros>
consolidacao() {
  i=0; membros=
  while [ "$i" -lt "$3" ]; do
    pr=$(($2 + i))
    REPORT=$4 PR=$pr agente "ag${n}m$i" "$HOME/m$i"
    membros="$membros${membros:+,}$pr"; i=$((i + 1))
  done
  printf '{"members":[%s]}' "$membros" >"$ORCH/consolidations/hades_$1.json"
}

case $n in
  # uso, alvos sem agente e fora do done
  000|001|002|003|004|005|006|007|008|011) repo "$caso" "$([ "$n" = 011 ] && echo git@github.com:acme/outro.git || echo git@github.com:acme/widget.git)" ;;
  009) repo "$caso"; printf '{nao' >"$ORCH/agents/ag$n.json" ;;
  010) repo "$caso"; printf '[1]' >"$ORCH/agents/ag$n.json" ;;
  012) repo "$caso"; agente "ag${n}m" "$HOME/m" ;;
  013) repo "$caso"; PR=9101 agente "ag${n}a" "$HOME/a"; REPORT='sem status\n' PR=9102 agente "ag${n}b" "$HOME/b"
       printf '{"members":[9101,9102]}' >"$ORCH/consolidations/hades_cons.json" ;;
  014) repo "$caso"; printf '{nao' >"$ORCH/consolidations/hades_ruim.json" ;;
  015|016)
    # mesmo agente em dois repositórios: a worktree do registro é a do caso 015; o 016 roda num repo sem origin
    if [ "$n" = 015 ]; then wt=$HOME/$caso; else repo "$caso" ""; wt=$HOME/wt; fi
    repo "$wt"; mkdir -p "$wt/.design-sync-reports"; printf 'STATUS: DONE\n' >"$wt/.design-sync-reports/report.md"
    reg="{\"name\":\"dup\",\"worktree\":\"$wt\",\"branch\":\"feature\",\"base\":\"main\",\"mode\":\"impl\",\"state\":\"active\"}"
    mkdir -p "$HOME/work/acme__outro/orchestration/agents"
    printf '%s' "$reg" >"$ORCH/agents/dup.json"; printf '%s' "$reg" >"$HOME/work/acme__outro/orchestration/agents/dup.json" ;;

  # auditoria status
  017) principal ;;
  018) REPORT='STATUS: PARKED — teto\n' principal ;;
  019) REPORT='' principal ;;
  020) SEM_REPORT=1 principal ;;
  021) principal ;;

  # auditoria gates
  022) REPORT='STATUS: DONE\n\nnada\n' principal ;;
  023) REPORT='STATUS: DONE\n\n- `gate-lock.sh run test --label zz-sem -- bun test` passou\n' principal ;;
  024) REPORT='STATUS: DONE\n\n- `gate-lock.sh run test --label bun-x -- bun test` verde\n' principal; linha test bun-x 0 ;;
  025) REPORT='STATUS: DONE\n\n- pela fila: run test --label bun-y ok\n' principal
       linha test bun-y 0; linha test bun-y 1 2026-01-02T00:00:00Z ;;
  026) REPORT='STATUS: DONE\n\n- pela fila: run lint --label bun-z passou\n' CREATED=2030-01-01T00:00:00Z principal; linha lint bun-z 0 ;;
  027) REPORT='STATUS: DONE\n\n- pela fila: run test --label bun-w passou\n' principal; linha test:integration bun-w 0 ;;
  028) REPORT='STATUS: DONE\n\n- pela fila: run test falhou, depois ok\n' principal ;;
  029) REPORT='STATUS: DONE\n\n## Gate (fila)\n\nrun build --label q passou\n' principal ;;
  030) REPORT='STATUS: DONE\n\n```\npela fila: run test passou\n```\n' principal ;;

  # auditoria capturas
  031) REPORT='STATUS: DONE\n\ncapturas: nenhuma\n' principal ;;
  032) REPORT='STATUS: DONE\n\n- captura `tela.png` da home\n' principal; printf x >"$caso/.design-sync-reports/tela.png" ;;
  033) REPORT='STATUS: DONE\n\n- captura `sumiu.png`\n' principal ;;
  034) REPORT='STATUS: DONE\n\n- screenshot docs/vazia.png\n' principal; mkdir "$caso/docs"; : >"$caso/docs/vazia.png" ;;
  035) REPORT='STATUS: DONE\n\n- captura nada/aqui/x.png\n' principal ;;
  036) REPORT='STATUS: DONE\n\n- capturas `<pagina>-{antes,depois}.png` e `a*.png`\n' principal ;;
  037) REPORT='STATUS: DONE\n\n- captura `perdida.webp`\n' principal; mkdir -p "$caso/deep/er"; printf x >"$caso/deep/er/perdida.webp" ;;
  038) REPORT='STATUS: DONE\n\n- captura `shots/um.png`, `dois.png`\n' principal
       mkdir "$caso/shots"; printf x >"$caso/shots/um.png"; printf y >"$caso/shots/dois.png" ;;
  039) REPORT="STATUS: DONE\n\n- captura $HOME/abs.png e ~/nao-existe-x.png\n" principal; printf x >"$HOME/abs.png" ;;

  # auditoria commits
  040) REPORT='STATUS: DONE\n\nnada\n' principal ;;
  041) principal; s=$(sha_citavel "$caso"); printf 'STATUS: DONE\n\n- commit `%s` na branch feature\n' "$s" >"$caso/.design-sync-reports/report.md" ;;
  042) REPORT='STATUS: DONE\n\n- commit `abc1234def` na branch feature\n' principal ;;
  043) REPORT='STATUS: DONE\n\n- commit `abc1234def` de skill/outra\n' principal ;;
  044) REPORT='STATUS: DONE\n\n- commit `abc1234def` em /opt/outro/repo\n' principal ;;
  045) principal; s=$(lateral "$caso" lateral 'l\n' l.txt lateral)
       printf 'STATUS: DONE\n\n- commit %s sobre a branch feature\n' "$s" >"$caso/.design-sync-reports/report.md" ;;
  046) principal; t=$(g "$caso" rev-parse --short 'HEAD^{tree}')
       printf 'STATUS: DONE\n\n- commit %s (árvore)\n' "$t" >"$caso/.design-sync-reports/report.md" ;;
  047) REPORT='STATUS: DONE\n\nversão abc1234def\n\n- commit deadbeef sem dígito? não: dead\n' principal ;;

  # auditoria escopo
  048) principal ;;
  049) BRIEF='# só texto\n' principal ;;
  050) BRIEF=$BRIEF_ESCOPO principal; arquivos "$caso" src/x.ts 1 docs/a.md 2 kit/y.sh 3 ;;
  051) BRIEF=$BRIEF_ESCOPO principal; arquivos "$caso" lib.sh 1 ;;
  052) BRIEF=$BRIEF_ESCOPO principal; arquivos "$caso" loop/bin/z.py 1 ;;
  053) BRIEF=$BRIEF_ESCOPO principal; arquivos "$caso" outro/a.txt 1 ;;
  054) BRIEF=$BRIEF_ESCOPO REPORT='STATUS: DONE\n\naditivo em outro/a.txt\n' principal; arquivos "$caso" outro/a.txt 1 ;;
  055) BRIEF=$BRIEF_ESCOPO BASE=naoexiste principal ;;
  056) BRIEF=$BRIEF_ESCOPO BASE= principal ;;
  057) BRIEF='**Não toque:** `run.sh`\n' principal; arquivos "$caso" sub/run.sh 1 ;;

  # auditoria perguntas
  058) principal ;;
  059) QUESTIONS='## Q1 — x\n\nctx\n' principal ;;
  060) QUESTIONS='## Q1 — x (respondida)\n' principal ;;
  061) QUESTIONS='## Q1 — x\n\n## Resposta do dono\n\nfaça\n' principal ;;
  062) REPORT='STATUS: ACTIVE\n' QUESTIONS='## Q1 — x\n' principal ;;
  063) longo=$(printf 'longo %.0s' $(seq 30)); q=; i=1
       while [ "$i" -le 8 ]; do q="$q${q:+\n}## Q$i — $longo\n"; i=$((i + 1)); done
       QUESTIONS=$q principal ;;

  # coerência registro (sem resposta de herdr: indisponível; gh pr list sem resposta no checks-rig era [])
  064) principal; viva; pr7 ;;
  065) WT=$HOME/fantasma principal; viva; prs '[]' ;;
  066) BRANCH=outra principal; viva; pr7 OPEN 7 outra ;;
  067) MODE=review BRANCH=outra principal; viva; prs '[]' ;;
  068) BASE=deadbeef1234 principal; viva; pr7 ;;
  069) BASE=origin/naoexiste principal; viva; pr7 ;;
  070) repo "$caso"; alheia=$(lateral "$caso" lat z z z); BASE=$alheia principal; viva; pr7 ;;
  071) BASE=feature principal; viva; pr7 ;;
  072) principal; pr7 ;;
  073) principal; pr7; resposta herdr agent-list nao-json ;;
  074) principal; pr7; resposta herdr agent-list '{}' ;;
  075) principal; viva w9:p9; pr7 ;;
  076) principal; pr7; herdr_json '{"name":"x","pane_id":"w1:p1"}' ;;
  077) principal; pr7; herdr_json "{\"name\":\"x\",\"cwd\":\"$HOME/$caso\"}" ;;
  078) principal; pr7; herdr_json '{"name":"x","pane_id":"w5:p5"}' ;;
  079) STATE=done principal; herdr_json ''; pr7 MERGED ;;
  080) principal; viva; prs '[]' ;;
  081) PR= principal; herdr_json ''; prs '[]' ;;
  082) principal; viva; pr7 OPEN 8 ;;
  083) principal; viva; pr7 CLOSED ;;
  084) principal; viva; pr7 MERGED ;;
  085) principal; viva; pr7 OPEN 7 outra ;;
  086) PR= principal; viva; pr7 OPEN 3 ;;
  087) principal; viva; resposta gh pr-list '' falhou 1 ;;
  088) principal; viva; prs lixo ;;
  089) MODE=skill principal; viva; prs '[]' ;;
  090) BRANCH= principal; viva; prs '[]' ;;

  # coerência plano
  091) principal ;;
  092) principal; printf '{nao' >"$ORCH/plan.json" ;;
  093) principal; printf '{}' >"$ORCH/plan.json" ;;
  094) principal; printf '{"path":"%s"}' "$HOME/sumiu-plano.md" >"$ORCH/plan.json" ;;
  095) principal; plano - ;;
  096) principal; plano '- outro agente em curso' ;;
  097) STATE=done principal; plano '- outro agente em curso' ;;
  098) principal; plano "- ag$n no w1:p1, porta 21290, PR #7" ;;
  099) principal; plano "- ag$n no w9:p9, porta 1234, PR #99" ;;
  100) principal; plano "- ag$n commit deadbee1234 feito" ;;
  101) principal; plano "- ag$n commit $(sha_citavel "$caso") feito" ;;
  102) principal; agente "ag${n}b" "$HOME/b"; plano "- ag$n PR #7; ag${n}b PR #55 w4:p4\n  continuação do item" ;;
  103) MODE=skill principal; plano '- ninguém' ;;

  # evidência: consolidação com 12 membros barrados
  104) repo "$caso"; consolidacao muitos 9300 12 'sem status\n' ;;

  *) echo "vigilia.sh: caso $n desconhecido" >&2; exit 1 ;;
esac
