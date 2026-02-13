"""List Fireblocks vault accounts using JWT authentication."""

import argparse
import hashlib
import json
import time
import uuid

import jwt
import requests


def sign_jwt(api_key: str, private_key: str, path: str, body: str = "") -> str:
    """Create a signed JWT for Fireblocks API authentication."""
    now = int(time.time())
    body_hash = hashlib.sha256(body.encode("utf-8")).hexdigest()

    payload = {
        "uri": path,
        "nonce": str(uuid.uuid4()),
        "iat": now,
        "exp": now + 30,
        "sub": api_key,
        "bodyHash": body_hash,
    }

    return jwt.encode(payload, private_key, algorithm="RS256")


def main():
    parser = argparse.ArgumentParser(description="List Fireblocks vault accounts")
    parser.add_argument("--api-key", required=True, help="Fireblocks API key")
    parser.add_argument("--secret-key-path", required=True, help="Path to the RSA private key file")
    args = parser.parse_args()

    with open(args.secret_key_path) as f:
        private_key = f.read()

    path = "/v1/vault/accounts_paged"
    token = sign_jwt(args.api_key, private_key, path)

    resp = requests.get(
        f"https://api.fireblocks.io{path}",
        headers={
            "X-API-Key": args.api_key,
            "Authorization": f"Bearer {token}",
        },
    )

    if not resp.ok:
        print(f"Error {resp.status_code}: {resp.text}")
    else:
        print(json.dumps(resp.json(), indent=2))


if __name__ == "__main__":
    main()
