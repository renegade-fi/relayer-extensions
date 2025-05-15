-- Remove the new `chain` column from all tables

-- First revert the primary key changes
ALTER TABLE "indexing_metadata" DROP CONSTRAINT IF EXISTS "indexing_metadata_pkey";
ALTER TABLE "indexing_metadata" ADD PRIMARY KEY ("key");

-- Then drop the chain columns
ALTER TABLE "fees" DROP COLUMN "chain";

ALTER TABLE "gas_wallets" DROP COLUMN "chain";

ALTER TABLE "hot_wallets" DROP COLUMN "chain";

ALTER TABLE "indexing_metadata" DROP COLUMN "chain";

ALTER TABLE "renegade_wallets" DROP COLUMN "chain";

