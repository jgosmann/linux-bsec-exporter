#!/bin/bash

set -o errexit -o nounset -o pipefail

host="$1"
path="$2"
shift 2
scp "$path" "$host":/tmp
filename=`basename "$path"`
ssh "$host" -tt -C "/tmp/$filename $@"
