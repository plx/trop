-- Exact schema emitted by the published trop 0.1.0 crate.
--
-- Crate checksum:
-- b7d3ed1d143b139c6fac47b197cf5d4f20d42459c1ee71c9fe8b634cae65a881
-- Embedded VCS commit:
-- eaceea6bc196fc5f787e2320bf6016e1a6f6bf88
CREATE TABLE metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE reservations (
    path TEXT NOT NULL,
    tag TEXT,
    port INTEGER NOT NULL UNIQUE,
    project TEXT,
    task TEXT,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    PRIMARY KEY (path, tag)
);

CREATE INDEX idx_reservations_port ON reservations(port);
CREATE INDEX idx_reservations_project ON reservations(project);
CREATE INDEX idx_reservations_last_used ON reservations(last_used_at);

INSERT INTO metadata (key, value) VALUES ('schema_version', '1');
