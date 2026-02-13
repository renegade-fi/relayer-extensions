"""Refill gas for all active gas wallets via the funds manager API."""

import argparse
import hmac
import hashlib
import json

import requests


def refill_gas(
    host: str,
    chain: str,
    amount: float,
    hmac_key_hex: str | None = None,
) -> dict:
    """Refill gas for all active gas wallets."""
    path = f"/custody/{chain}/gas/refill-gas"
    body = json.dumps(
        {"amount": amount},
        separators=(",", ":"),
    )

    headers = {"Content-Type": "application/json"}

    if hmac_key_hex:
        hmac_key = bytes.fromhex(hmac_key_hex)
        message = b"POST" + path.encode() + body.encode()
        signature = hmac.new(hmac_key, message, hashlib.sha256).hexdigest()
        headers["X-Signature"] = signature

    resp = requests.post(host + path, headers=headers, data=body, timeout=300)
    if not resp.ok:
        print(f"Error {resp.status_code}: {resp.text}")
    resp.raise_for_status()
    return resp.json()


def main():
    parser = argparse.ArgumentParser(description="Refill gas for all active gas wallets")
    parser.add_argument("--host", required=True, help="Funds manager host, e.g. http://localhost:3000")
    parser.add_argument("--chain", default="ethereum-sepolia", help="Chain name (default: ethereum-sepolia)")
    parser.add_argument("--amount", type=float, default=0.01, help="Amount of ETH to top up each wallet to (default: 0.01)")
    parser.add_argument("--hmac-key", default=None, help="HMAC key as hex string (omit if auth is disabled)")
    args = parser.parse_args()

    print(f"Refilling gas wallets on {args.chain} to {args.amount} ETH each...")
    result = refill_gas(
        host=args.host,
        chain=args.chain,
        amount=args.amount,
        hmac_key_hex=args.hmac_key,
    )
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
