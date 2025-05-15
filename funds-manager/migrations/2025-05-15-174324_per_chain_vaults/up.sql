-- Migrate the hot_wallets table to use chain-specific vault names.
-- Currently, all vaults in the table are Arbitrum vaults,
-- and the Arbitrum Gas vault is already correctly named.

UPDATE hot_wallets
SET vault = 'Arbitrum ' || vault
WHERE vault != 'Arbitrum Gas';
