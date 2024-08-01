-- Create a table to store gas wallets
CREATE TABLE gas_wallets (
    id UUID PRIMARY KEY,
    address TEXT NOT NULL UNIQUE,
    peer_id TEXT,
    status TEXT NOT NULL DEFAULT 'inactive',
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);