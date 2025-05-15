-- Remove the "Arbitrum " prefix from the hot_wallets vault column

UPDATE hot_wallets
SET vault = SUBSTRING(vault FROM 10)
WHERE vault LIKE 'Arbitrum %' AND vault != 'Arbitrum Gas';
