-- Stores fees and index by mint, amount
CREATE TABLE fees(
    id SERIAL PRIMARY KEY,
    tx_hash TEXT NOT NULL UNIQUE,
    mint TEXT NOT NULL,
    amount NUMERIC NOT NULL,
    blinder NUMERIC NOT NULL,
    receiver TEXT NOT NULL,
    redeemed BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX idx_fees_mint ON fees(mint);
CREATE INDEX idx_fees_amount ON fees(amount);
