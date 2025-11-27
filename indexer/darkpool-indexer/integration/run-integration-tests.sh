#!/bin/bash
set -Eeuo pipefail

# Use BuildKit
export DOCKER_BUILDKIT=1

# Assume this script is being invoked from the repository root
compose_file="indexer/darkpool-indexer/integration/docker-compose.yml"

docker compose \
    --file $compose_file \
    up \
    --remove-orphans \
    --build \
    --force-recreate \
    --abort-on-container-exit \
    --abort-on-container-failure \
    --timeout 1
