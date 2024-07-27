-- Create a table for storing hot wallets
CREATE TABLE hot_wallets(
    id UUID PRIMARY KEY,
    secret_id TEXT NOT NULL, -- The AWS Secrets Manager secret that holds the hot wallet's key
    vault TEXT NOT NULL,     -- The fireblocks vault that the hot wallet writes back to
    address TEXT NOT NULL    -- The Arbitrum address of the hot wallet
);

--- Rename the wallets table to renegade_wallets for clarity
ALTER TABLE wallets RENAME TO renegade_wallets;
