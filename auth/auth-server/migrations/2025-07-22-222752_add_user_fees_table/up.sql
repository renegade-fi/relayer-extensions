-- Create the user_fees table with compound primary key and foreign key constraint
CREATE TABLE user_fees (
    id UUID NOT NULL,
    asset VARCHAR NOT NULL,
    fee REAL NOT NULL,
    PRIMARY KEY (id, asset),
    FOREIGN KEY (id) REFERENCES api_keys(id) ON DELETE CASCADE
);
