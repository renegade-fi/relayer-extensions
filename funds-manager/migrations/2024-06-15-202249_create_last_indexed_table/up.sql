-- Create the table that stores indexing metadata
CREATE TABLE indexing_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Insert a row with the latest block number set to zero
INSERT INTO indexing_metadata (key, value) VALUES ('latest_block', '0');
