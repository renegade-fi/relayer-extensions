"""Create a hot wallet for Ethereum Sepolia via the funds manager API."""

import argparse
import hmac
import hashlib
import json
import uuid

import requests


def create_hot_wallet(
    host: str,
    chain: str,
    vault: str,
    internal_wallet_id: str,
    hmac_key_hex: str | None = None,
) -> dict:
    """Create a hot wallet via the funds manager API."""
    path = f"/custody/{chain}/hot-wallets"
    body = json.dumps(
        {"vault": vault, "internal_wallet_id": internal_wallet_id},
        separators=(",", ":"),
    )

    headers = {"Content-Type": "application/json"}

    if hmac_key_hex:
        hmac_key = bytes.fromhex(hmac_key_hex)
        # V1 auth: HMAC over method + path + body
        message = b"POST" + path.encode() + body.encode()
        signature = hmac.new(hmac_key, message, hashlib.sha256).hexdigest()
        headers["X-Signature"] = signature

    resp = requests.post(host + path, headers=headers, data=body)
    if not resp.ok:
        print(f"Error {resp.status_code}: {resp.text}")
    resp.raise_for_status()
    return resp.json()


def main():
    parser = argparse.ArgumentParser(description="Create a hot wallet")
    parser.add_argument("--host", required=True, help="Funds manager host, e.g. http://localhost:3000")
    parser.add_argument("--chain", default="ethereum-sepolia", help="Chain name (default: ethereum-sepolia)")
    parser.add_argument("--vault", default="Ethereum Sepolia Hot Wallet", help="Vault name (default: Ethereum Sepolia Hot Wallet)")
    parser.add_argument("--internal-wallet-id", default=None, help="Internal wallet UUID (auto-generated if omitted)")
    parser.add_argument("--hmac-key", default=None, help="HMAC key as hex string (omit if auth is disabled)")
    args = parser.parse_args()

    internal_wallet_id = args.internal_wallet_id or str(uuid.uuid4())
    print(f"Using internal wallet ID: {internal_wallet_id}")

    result = create_hot_wallet(
        host=args.host,
        chain=args.chain,
        vault=args.vault,
        internal_wallet_id=internal_wallet_id,
        hmac_key_hex=args.hmac_key,
    )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
