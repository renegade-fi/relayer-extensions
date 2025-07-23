-- Create the asset_default_fees table with asset as primary key
CREATE TABLE asset_default_fees (
    asset VARCHAR PRIMARY KEY,
    fee REAL NOT NULL
);
