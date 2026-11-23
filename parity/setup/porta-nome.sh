. "$(dirname "$0")/porta-lib.sh"
script() {
  repo "$1"; mkdir -p "$1/kit"
  printf '#!/usr/bin/env bash\n%s\n' "$2" >"$1/kit/skill-release.sh"
}
repo solto
script limpo "echo limpo; exit 0"
script termo "echo 'a.md:1: termo'; exit 1"
script saiu-1 "exit 1"
script sem-perfil "echo 'guarda de nome: nenhum perfil instalado'"
script vazio "exit 0"
