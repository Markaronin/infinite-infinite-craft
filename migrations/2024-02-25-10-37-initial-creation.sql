CREATE TABLE IF NOT EXISTS elements (
    result TEXT PRIMARY KEY NOT NULL,
    emoji TEXT NOT NULL,
    is_new BOOLEAN NOT NULL
);
CREATE TABLE IF NOT EXISTS pairs (
    first TEXT NOT NULL,
    second TEXT NOT NULL,
    result TEXT,
    PRIMARY KEY (first, second)
);