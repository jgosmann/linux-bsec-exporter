#!/bin/bash

set -o errexit -o nounset -o pipefail

scp "$2" "$1":/tmp
filename=`basename "$2"`
ssh "$1" -tt -C "/tmp/$filename"
