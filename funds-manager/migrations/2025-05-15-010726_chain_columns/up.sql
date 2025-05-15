-- Add a `chain` column to all tables, defaulting to `arbitrum`
-- (as we previously have only had the funds manager servicing our Arbitrum deployment)
ALTER TABLE "fees" ADD COLUMN "chain" TEXT NOT NULL DEFAULT 'arbitrum';

ALTER TABLE "gas_wallets" ADD COLUMN "chain" TEXT NOT NULL DEFAULT 'arbitrum';

ALTER TABLE "hot_wallets" ADD COLUMN "chain" TEXT NOT NULL DEFAULT 'arbitrum';

ALTER TABLE "indexing_metadata" ADD COLUMN "chain" TEXT NOT NULL DEFAULT 'arbitrum';

-- Drop existing primary key first
ALTER TABLE "indexing_metadata" DROP CONSTRAINT IF EXISTS "indexing_metadata_pkey";
-- Add new composite primary key
ALTER TABLE "indexing_metadata" ADD PRIMARY KEY ("key", "chain");

ALTER TABLE "renegade_wallets" ADD COLUMN "chain" TEXT NOT NULL DEFAULT 'arbitrum';
