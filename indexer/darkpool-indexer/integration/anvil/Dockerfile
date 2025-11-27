FROM ghcr.io/foundry-rs/foundry:latest

# Add curl
USER root
RUN apt update && apt install -y curl

# Install huffc
RUN curl -L get.huff.sh | bash
RUN ~/.huff/bin/huffup

# Add the huff binaries to the system's PATH
ENV PATH="/root/.huff/bin:${PATH}"

# Clone the contracts repo
# We ADD the version file first to cache the git clone appropriately
ADD https://api.github.com/repos/renegade-fi/renegade-contracts/git/refs/heads/main /renegade-contracts-version.json
WORKDIR /sources
RUN git clone --recurse-submodules https://github.com/renegade-fi/renegade-contracts.git

WORKDIR /sources/renegade-contracts

# Bring in the contract deployment script
COPY deploy_contracts.sh /deploy_contracts.sh
RUN chmod +x /deploy_contracts.sh

# Run the contract deployment script, so that the Anvil state snapshot
# is included in the image
RUN /deploy_contracts.sh

ENTRYPOINT ["anvil", "--state", "/anvil-state.json"]