#!/bin/sh

set -eu

# shellcheck disable=SC2046
cd $(dirname "$0") || exit

export DATABASE_URL=mysql://ceer:ceer@127.0.0.1:3306/ceer

refinery migrate -e DATABASE_URL -p ../rdb-migration/migrations