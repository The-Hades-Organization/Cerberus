. "$(dirname "$0")/porta-lib.sh"
repo up-feature; upstream up-feature feature
repo up-outra; upstream up-outra outra
repo solto
repo destacado; g destacado checkout -q --detach
resposta gh pr-view "" "sem pr" 1
