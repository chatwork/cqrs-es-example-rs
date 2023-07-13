#!/bin/sh

set -eu

# shellcheck disable=SC2046
cd $(dirname "$0") || exit

if [[ ! -e ../../env.sh ]]; then
    echo "env.sh is not found."
    exit 1
fi

# shellcheck disable=SC2034
OUTPUT_ENV=1

source ../../env.sh

LOCAL_REPO_NAME=j5ik2o/cqrs-es-example-rs-read-model-updater
TAG=latest
LOCAL_URI=${LOCAL_REPO_NAME}:${TAG}
LOCAL_AMD64_URI=${LOCAL_REPO_NAME}:${TAG}-amd64

pushd ../../

ECR_BASE_URL=$(./tools/terraform/terraform-output.sh -raw ecr_read_model_updater_repository_url)

popd

REMOTE_MANIFEST_URI=${ECR_BASE_URL}:${TAG}
REMOTE_AMD64_URI=${ECR_BASE_URL}:${TAG}-amd64

echo ">>> ecr login"
aws --profile ${AWS_PROFILE} ecr get-login-password --region ${AWS_REGION} | docker login --username AWS --password-stdin ${ECR_BASE_URL}

echo ">>> docker tag"
docker tag ${LOCAL_AMD64_URI} ${REMOTE_AMD64_URI}

echo ">>> docker push"
docker push ${REMOTE_AMD64_URI}

echo ">>> docker manifest create"
docker manifest create --amend ${REMOTE_MANIFEST_URI} ${REMOTE_AMD64_URI}
echo ">>> docker manifest annotate"
docker manifest annotate --arch amd64 ${REMOTE_MANIFEST_URI} $REMOTE_AMD64_URI
echo ">>> docker manifest inspect"
docker manifest inspect ${REMOTE_MANIFEST_URI}
echo ">>> docker manifest push"
docker manifest push ${REMOTE_MANIFEST_URI}