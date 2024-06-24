-- Create a table for storing wallets and the mints they hold
-- The `secret_id` is the id of the AWS Secrets Manager secret that holds recovery information for the wallet
CREATE TABLE wallets (
    id UUID PRIMARY KEY,
    mints TEXT[],
    secret_id TEXT
);
