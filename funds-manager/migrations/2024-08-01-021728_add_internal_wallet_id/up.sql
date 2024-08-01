-- Add internal_wallet_id column as UUID
ALTER TABLE hot_wallets ADD COLUMN internal_wallet_id UUID;

-- Generate default UUID values for existing rows
UPDATE hot_wallets
SET internal_wallet_id = gen_random_uuid()
WHERE internal_wallet_id IS NULL;

-- Make the column NOT NULL
ALTER TABLE hot_wallets
ALTER COLUMN internal_wallet_id SET NOT NULL;