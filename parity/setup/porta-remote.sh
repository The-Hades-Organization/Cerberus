. "$(dirname "$0")/porta-lib.sh"
repo destacado; g destacado checkout -q --detach
repo sem-remote
repo pushremote; g pushremote config branch.feature.pushRemote no_push
repo pushurl; g pushurl config branch.feature.remote origin; g pushurl remote set-url --push origin no_push
repo fantasma; g fantasma config branch.feature.pushRemote fantasma
repo ok; g ok config branch.feature.remote origin
