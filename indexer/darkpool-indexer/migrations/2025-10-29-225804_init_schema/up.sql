-- Defines the initial set of tables for the darkpool indexer database.

-- BALANCES --

-- Stores darkpool balances
CREATE TABLE "balances"(
	"identifier_seed" NUMERIC NOT NULL PRIMARY KEY CHECK (identifier_seed >= 0),
	"account_id" UUID NOT NULL,
	"active" BOOL NOT NULL,
	"mint" TEXT NOT NULL,
	"owner_address" TEXT NOT NULL,
	"one_time_key" TEXT NOT NULL,
	"protocol_fee" NUMERIC NOT NULL CHECK (protocol_fee >= 0),
	"relayer_fee" NUMERIC NOT NULL CHECK (relayer_fee >= 0),
	"amount" NUMERIC NOT NULL CHECK (amount >= 0),
	"allow_public_fills" BOOL NOT NULL
);

-- Indexes a balance by its account ID & active flag
CREATE INDEX "idx_balances_account_id_active" ON "balances" ("account_id", "active");

-- EXPECTED STATE OBJECTS --

-- Stores information about state objects which are expected to be created
CREATE TABLE "expected_state_objects"(
	"nullifier" NUMERIC NOT NULL PRIMARY KEY CHECK (nullifier >= 0),
	"account_id" UUID NOT NULL,
	"owner_address" TEXT NOT NULL,
	"identifier_seed" NUMERIC NOT NULL CHECK (identifier_seed >= 0),
	"encryption_seed" NUMERIC NOT NULL CHECK (encryption_seed >= 0)
);

-- PROCESSED NULLIFIERS --

-- Stores nullifiers which have already been processed
CREATE TABLE "processed_nullifiers"(
	"nullifier" NUMERIC NOT NULL PRIMARY KEY CHECK (nullifier >= 0),
	"block_number" NUMERIC NOT NULL CHECK (block_number >= 0)
);

-- INTENTS --

-- Stores darkpool intents
CREATE TABLE "intents"(
	"identifier_seed" NUMERIC NOT NULL PRIMARY KEY CHECK (identifier_seed >= 0),
	"account_id" UUID NOT NULL,
	"active" BOOL NOT NULL,
	"input_mint" TEXT NOT NULL,
	"output_mint" TEXT NOT NULL,
	"owner_address" TEXT NOT NULL,
	"min_price" NUMERIC NOT NULL CHECK (min_price >= 0),
	"input_amount" NUMERIC NOT NULL CHECK (input_amount >= 0),
	"matching_pool" TEXT NOT NULL,
	"allow_external_matches" BOOL NOT NULL,
	"min_fill_size" NUMERIC NOT NULL CHECK (min_fill_size >= 0),
	"precompute_cancellation_proof" BOOL NOT NULL
);

-- Indexes an intent by its account ID & active flag
CREATE INDEX "idx_intents_account_id_active" ON "intents" ("account_id", "active");

-- MASTER VIEW SEEDS --

-- Stores users' master view seeds
CREATE TABLE "master_view_seeds"(
	"account_id" UUID NOT NULL PRIMARY KEY,
	"owner_address" TEXT NOT NULL,
	"seed" NUMERIC NOT NULL CHECK (seed >= 0)
);

-- GENERIC STATE OBJECTS --

-- An enum representing the type of a generic state object
CREATE TYPE "object_type" AS ENUM ('intent', 'balance');

-- Stores generic state objects
CREATE TABLE "generic_state_objects"(
	"identifier_seed" NUMERIC NOT NULL PRIMARY KEY CHECK (identifier_seed >= 0),
	"account_id" UUID NOT NULL,
	"active" BOOL NOT NULL,
	"object_type" "object_type" NOT NULL,
	"nullifier" NUMERIC NOT NULL CHECK (nullifier >= 0),
	"version" NUMERIC NOT NULL CHECK (version >= 0),
	"encryption_seed" NUMERIC NOT NULL CHECK (encryption_seed >= 0),
	"encryption_cipher_index" NUMERIC NOT NULL CHECK (encryption_cipher_index >= 0),
	"owner_address" TEXT NOT NULL,
	"public_shares" NUMERIC[] NOT NULL CHECK (array_position(public_shares, NULL) IS NULL AND 0 <= ALL(public_shares)),
	"private_shares" NUMERIC[] NOT NULL CHECK (array_position(private_shares, NULL) IS NULL AND 0 <= ALL(private_shares))
);

-- Indexes a generic state object by its account ID & active flag
CREATE INDEX "idx_generic_state_objects_account_id_active" ON "generic_state_objects" ("account_id", "active");

-- Indexes a generic state object by its nullifier
CREATE INDEX "idx_generic_state_objects_nullifier" ON "generic_state_objects" ("nullifier");

