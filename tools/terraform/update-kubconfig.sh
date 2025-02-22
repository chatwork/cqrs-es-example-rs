#!/usr/bin/env bash

set -eu

# shellcheck disable=SC2046
cd $(dirname "$0") || exit

if [[ ! -e ../../env.sh ]]; then
    echo "env.sh is not found."
    exit 1
fi

# shellcheck disable=SC2034
OUTPUT_ENV=1

source ./terraform-env.sh

NAME=$(terraform output --state="${TF_STATE_NAME}" -raw eks_cluster_name)

# shellcheck disable=SC2046
aws --profile ${AWS_PROFILE} eks update-kubeconfig --name ${NAME}
