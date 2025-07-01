-- Drop the whitelist flag
ALTER TABLE api_keys DROP COLUMN rate_limit_whitelisted;
