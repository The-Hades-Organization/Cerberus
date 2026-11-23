. "$(dirname "$0")/porta-lib.sh"
repo main; upstream main main
repo release; upstream release release
repo feature; upstream feature feature
repo falha git@github.com:acme/falha.git; upstream falha feature
repo solto-origin solto; upstream solto-origin feature
repo solto
repo destacado; g destacado checkout -q --detach
resposta gh api-repos/acme/widget/branches/release true
resposta gh api-repos/acme/widget/branches/feature false
resposta gh api-repos/acme/falha/branches/feature "" "HTTP 404" 1
