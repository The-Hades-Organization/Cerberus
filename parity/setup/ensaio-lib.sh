# Biblioteca dos setups do Ensaio (molde: mkPack de loop/test/ts/cerberus-ensaio.test.ts). `pacote <corpo>` monta em
# $HOME/pack um pacote git com env/post-install.sh executável e o $D em $HOME/data; autor e data fixos, então o commit
# sai com o sha que a matriz cita.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
export GIT_AUTHOR_DATE='2026-01-01T00:00:00Z' GIT_COMMITTER_DATE='2026-01-01T00:00:00Z'
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
set -eu

pacote() {
  p="$HOME/pack"
  mkdir -p "$p/env" "$HOME/data"
  git -C "$p" init -q -b main
  printf '{"name":"ens","env":{"pack":"env"}}' >"$p/profile.json"
  printf '{"name":"ens"}' >"$p/env/plugin.json"
  printf '#!/usr/bin/env bash\n%s\n' "$1" >"$p/env/post-install.sh"
  chmod 755 "$p/env/post-install.sh"
  git -C "$p" add -A
  git -C "$p" commit -qm v1
}
