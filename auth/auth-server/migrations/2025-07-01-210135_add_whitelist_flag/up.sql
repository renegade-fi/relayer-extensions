-- Add the whitelist flag
ALTER TABLE api_keys ADD COLUMN rate_limit_whitelisted BOOLEAN NOT NULL DEFAULT FALSE;
