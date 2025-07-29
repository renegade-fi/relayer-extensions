-- Create an index for faster retrieval of unredeemed fee totals
CREATE INDEX idx_redeemed_by_mint_chain ON fees (chain, redeemed, mint) INCLUDE (amount);
