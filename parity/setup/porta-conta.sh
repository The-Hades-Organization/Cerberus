. "$(dirname "$0")/porta-lib.sh"
perfil() {
  mkdir -p "packs/$1"
  printf '{"name":"%s","identity":{"repo":"acme/%s","gh":%s}}' "$1" "$1" "$2" >"packs/$1/profile.json"
  repo "$1" "git@github.com:acme/$1.git"
}
repo sem-perfil
perfil acme-1 '{"user":"Dev"}'
perfil acme-2 '{"user":"dev"}'
perfil acme-3 '{"tokenEnv":"TOKEN_DO_TESTE"}'
perfil acme-4 '{"user":"dev","tokenEnv":"TOKEN_DO_TESTE"}'
resposta gh api-user "" "sem login" 1
