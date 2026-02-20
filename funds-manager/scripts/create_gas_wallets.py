"""Create gas wallets via the funds manager API."""

import argparse
import hmac
import hashlib
import json

import requests


def create_gas_wallet(
    host: str,
    chain: str,
    hmac_key_hex: str | None = None,
) -> dict:
    """Create a single gas wallet via the funds manager API."""
    path = f"/custody/{chain}/gas-wallets"
    body = ""

    headers = {"Content-Type": "application/json"}

    if hmac_key_hex:
        hmac_key = bytes.fromhex(hmac_key_hex)
        message = b"POST" + path.encode() + body.encode()
        signature = hmac.new(hmac_key, message, hashlib.sha256).hexdigest()
        headers["X-Signature"] = signature

    resp = requests.post(host + path, headers=headers, data=body)
    if not resp.ok:
        print(f"Error {resp.status_code}: {resp.text}")
    resp.raise_for_status()
    return resp.json()


def main():
    parser = argparse.ArgumentParser(description="Create gas wallets")
    parser.add_argument("--host", required=True, help="Funds manager host, e.g. http://localhost:3000")
    parser.add_argument("--chain", default="ethereum-sepolia", help="Chain name (default: ethereum-sepolia)")
    parser.add_argument("-n", "--count", type=int, default=1, help="Number of gas wallets to create (default: 1)")
    parser.add_argument("--hmac-key", default=None, help="HMAC key as hex string (omit if auth is disabled)")
    args = parser.parse_args()

    for i in range(args.count):
        result = create_gas_wallet(
            host=args.host,
            chain=args.chain,
            hmac_key_hex=args.hmac_key,
        )
        print(f"[{i + 1}/{args.count}] Created gas wallet: {result['address']}")

    print(f"Done. Created {args.count} gas wallet(s).")


if __name__ == "__main__":
    main()
