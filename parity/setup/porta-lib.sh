# Biblioteca dos setups da matriz da Porta (molde: loop/test/ts/lib/checks-rig.ts do Hades-Skill). Roda no HOME do
# lado (sh, cwd = HOME). Git sem config do usuário e com data fixa: os dois lados geram os mesmos commits.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
export GIT_AUTHOR_DATE='2026-01-01T00:00:00Z' GIT_COMMITTER_DATE='2026-01-01T00:00:00Z'
set -eu

g() { d=$1; shift; git -C "$d" "$@"; }

# repo <pasta> [origin]: branch feature, um commit; origin padrão git@github.com:acme/widget.git, "" = sem origin.
repo() {
  d=$1; o=${2-git@github.com:acme/widget.git}
  mkdir -p "$d"
  g "$d" init -q -b feature
  g "$d" config user.email t@t
  g "$d" config user.name t
  [ -n "$o" ] && g "$d" remote add origin "$o"
  printf 'um\n' >"$d/a.txt"
  g "$d" add -A
  g "$d" commit -qm um
}

# upstream <pasta> <branch>: branch.feature.remote = origin e branch.feature.merge = refs/heads/<branch>.
upstream() {
  g "$1" config branch.feature.remote origin
  g "$1" config branch.feature.merge "refs/heads/$2"
}

# resposta <programa> <arquivo> <stdout> [stderr rc]: resposta dos stubs de parity/stubs ($HOME/fix/<programa>/...).
resposta() {
  f="$HOME/fix/$1/$2"
  mkdir -p "$(dirname "$f")"
  rm -f "$f.out" "$f.err" "$f.rc"
  [ -n "$3" ] && printf '%s\n' "$3" >"$f.out"
  if [ $# -ge 5 ]; then printf '%s\n' "$4" >"$f.err"; printf '%s\n' "$5" >"$f.rc"; fi
  return 0
}

# token do GitHub inventado com o formato da regex, montado em partes para não casar com varredura de segredos.
TOKEN_GH="gh""p_$(printf 'a1B2%.0s' 1 2 3 4 5 6 7 8 9)"
CHAVE_PRIVADA="-----BEGIN RSA PRIV""ATE KEY-----"
