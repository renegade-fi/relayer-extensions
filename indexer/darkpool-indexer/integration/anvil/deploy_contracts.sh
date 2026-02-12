#!/bin/bash
set -Eeuo pipefail

# Constants
DEPLOY_SCRIPT="script/v2/DeployDev.s.sol:DeployDevScript"
ANVIL_RPC_URL="http://localhost:8545"
ANVIL_PKEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"


# Spawn a local Anvil node which will snapshot its state upon exit
anvil \
    --disable-code-size-limit \
    --state /anvil-state.json \
    &

ANVIL_PID=$!

# Add a small sleep to ensure the Anvil node ready
sleep 1

# Run the deployment script
forge script \
    $DEPLOY_SCRIPT \
    --ffi \
    --rpc-url $ANVIL_RPC_URL \
    --private-key $ANVIL_PKEY \
    --broadcast \
    --disable-code-size-limit

# Copy deployments file to the expected location
cp deployments.devnet.json /deployments.json

# Send a termination signal to the Anvil node to snapshot its state
kill -TERM $ANVIL_PID

# Wait for the Anvil node to exit
wait $ANVIL_PID
