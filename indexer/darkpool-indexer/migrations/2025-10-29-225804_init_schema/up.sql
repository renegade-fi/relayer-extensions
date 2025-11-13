-- Defines the initial set of tables for the darkpool indexer database.
-- All numeric columns represent unsigned 256-bit integers. As such, we constrain all to be non-negative, have a precision of 78 (# of digits in 2^256), and a scale of 0 (no fractional component)

-- BALANCES --

-- Stores darkpool balances
CREATE TABLE "balances"(
	"recovery_stream_seed" NUMERIC(78) NOT NULL PRIMARY KEY CHECK (recovery_stream_seed >= 0),
	"version" BIGINT NOT NULL CHECK (version >= 0),
	"share_stream_seed" NUMERIC(78) NOT NULL CHECK (share_stream_seed >= 0),
	"share_stream_index" BIGINT NOT NULL CHECK (share_stream_index >= 0),
	"nullifier" NUMERIC(78) NOT NULL CHECK (nullifier >= 0),
	"public_shares" NUMERIC(78)[] NOT NULL CHECK (array_position(public_shares, NULL) IS NULL AND 0 <= ALL(public_shares)),
	"mint" TEXT NOT NULL,
	"owner_address" TEXT NOT NULL,
	"relayer_fee_recipient" TEXT NOT NULL,
	"one_time_authority" TEXT NOT NULL,
	"protocol_fee" NUMERIC(78) NOT NULL CHECK (protocol_fee >= 0),
	"relayer_fee" NUMERIC(78) NOT NULL CHECK (relayer_fee >= 0),
	"amount" NUMERIC(78) NOT NULL CHECK (amount >= 0),
	"account_id" UUID NOT NULL,
	"active" BOOL NOT NULL
);

-- Indexes a balance by its nullifier
CREATE INDEX "idx_balances_nullifier" ON "balances" ("nullifier");

-- Indexes a balance by its account ID & active flag
CREATE INDEX "idx_balances_account_id_active" ON "balances" ("account_id", "active");

-- EXPECTED STATE OBJECTS --

-- Stores information about state objects which are expected to be created
CREATE TABLE "expected_state_objects"(
	"recovery_id" NUMERIC(78) NOT NULL PRIMARY KEY CHECK (recovery_id >= 0),
	"account_id" UUID NOT NULL,
	"recovery_stream_seed" NUMERIC(78) NOT NULL CHECK (recovery_stream_seed >= 0),
	"share_stream_seed" NUMERIC(78) NOT NULL CHECK (share_stream_seed >= 0)
);

-- PROCESSED NULLIFIERS --

-- Stores nullifiers which have already been processed
CREATE TABLE "processed_nullifiers"(
	"nullifier" NUMERIC(78) NOT NULL PRIMARY KEY CHECK (nullifier >= 0),
	"block_number" BIGINT NOT NULL CHECK (block_number >= 0)
);

-- PROCESSED RECOVERY IDs --

-- Stores recovery IDs which have already been processed
CREATE TABLE "processed_recovery_ids"(
	"recovery_id" NUMERIC(78) NOT NULL PRIMARY KEY CHECK (recovery_id >= 0),
	"block_number" BIGINT NOT NULL CHECK (block_number >= 0)
);

-- INTENTS --

-- Stores darkpool intents
CREATE TABLE "intents"(
	"recovery_stream_seed" NUMERIC(78) NOT NULL PRIMARY KEY CHECK (recovery_stream_seed >= 0),
	"version" BIGINT NOT NULL CHECK (version >= 0),
	"share_stream_seed" NUMERIC(78) NOT NULL CHECK (share_stream_seed >= 0),
	"share_stream_index" BIGINT NOT NULL CHECK (share_stream_index >= 0),
	"nullifier" NUMERIC(78) NOT NULL CHECK (nullifier >= 0),
	"public_shares" NUMERIC(78)[] NOT NULL CHECK (array_position(public_shares, NULL) IS NULL AND 0 <= ALL(public_shares)),
	"input_mint" TEXT NOT NULL,
	"output_mint" TEXT NOT NULL,
	"owner_address" TEXT NOT NULL,
	"min_price" NUMERIC(78) NOT NULL CHECK (min_price >= 0),
	"input_amount" NUMERIC(78) NOT NULL CHECK (input_amount >= 0),
	"account_id" UUID NOT NULL,
	"active" BOOL NOT NULL,
	"matching_pool" TEXT NOT NULL,
	"allow_external_matches" BOOL NOT NULL,
	"min_fill_size" NUMERIC(78) NOT NULL CHECK (min_fill_size >= 0),
	"precompute_cancellation_proof" BOOL NOT NULL
);

-- Indexes an intent by its nullifier
CREATE INDEX "idx_intents_nullifier" ON "intents" ("nullifier");

-- Indexes an intent by its account ID & active flag
CREATE INDEX "idx_intents_account_id_active" ON "intents" ("account_id", "active");

-- MASTER VIEW SEEDS --

-- Stores users' master view seeds
CREATE TABLE "master_view_seeds"(
	"account_id" UUID NOT NULL PRIMARY KEY,
	"owner_address" TEXT NOT NULL,
	"seed" NUMERIC(78) NOT NULL CHECK (seed >= 0),
	"recovery_seed_csprng_index" BIGINT NOT NULL CHECK (recovery_seed_csprng_index >= 0),
	"share_seed_csprng_index" BIGINT NOT NULL CHECK (share_seed_csprng_index >= 0)
);
