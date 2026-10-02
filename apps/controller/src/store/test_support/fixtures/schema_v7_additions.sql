CREATE TABLE network_identity(id INTEGER PRIMARY KEY CHECK(id=1),network_id TEXT NOT NULL UNIQUE CHECK(length(network_id)=32),name TEXT NOT NULL,dns TEXT NOT NULL);
PRAGMA user_version=7;
