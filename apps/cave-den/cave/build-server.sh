#!/usr/bin/env bash
# The Cave's modules for the den's server: copies cave/server/{lib,test} into
# SERVER/extra, which the den compiles in when it is there (mix.exs). Then run
# the den with
#
#   DEN_GATE=Den.Gate.Zyn DEN_NODE_URL=… DEN_COLLECTION=…
#   DEN_EXTRA_CHILDREN=Den.Cave.Proposals
#   DEN_ROOMS=general:General,diggers:Diggers,lookouts:Lookouts,lanterns:Lanterns
#
#   cave/build-server.sh [SERVER]     (default ../server)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SERVER=${1:-$HERE/../server}
rm -rf "$SERVER/extra/lib" "$SERVER/extra/test" && mkdir -p "$SERVER/extra"
cp -R "$HERE/server/lib" "$HERE/server/test" "$SERVER/extra/"
echo "Cave modules in $SERVER/extra"
