-- Create a table for caching wallet compliance information
CREATE TABLE IF NOT EXISTS wallet_compliance (
    address TEXT PRIMARY KEY,
    is_compliant BOOLEAN NOT NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMP NOT NULL DEFAULT NOW() + INTERVAL '1 year'
);
