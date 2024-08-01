-- Create a table to store gas wallets
CREATE TABLE gas_wallets (
    id UUID PRIMARY KEY,
    address TEXT NOT NULL UNIQUE,
    peer_id TEXT,
    active BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);