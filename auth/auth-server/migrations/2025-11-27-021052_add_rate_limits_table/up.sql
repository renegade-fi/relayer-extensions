-- Create the rate_limit_method enum type
CREATE TYPE rate_limit_method AS ENUM ('quote', 'assemble');

-- Create the rate_limits table with compound primary key and foreign key constraint
CREATE TABLE rate_limits (
    api_key_id UUID NOT NULL,
    method rate_limit_method NOT NULL,
    requests_per_minute INTEGER NOT NULL,
    PRIMARY KEY (api_key_id, method),
    FOREIGN KEY (api_key_id) REFERENCES api_keys(id) ON DELETE CASCADE
);
