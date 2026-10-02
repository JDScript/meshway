CREATE TABLE connection_runtime_leases(node_id TEXT PRIMARY KEY REFERENCES nodes(id),instance_id TEXT NOT NULL,epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 4294967295));
CREATE TABLE connection_runtime_reports(node_id TEXT PRIMARY KEY REFERENCES connection_runtime_leases(node_id),sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 4294967295),body BLOB NOT NULL,reported_at TEXT NOT NULL);
CREATE TABLE mesh_access_source(id INTEGER PRIMARY KEY CHECK(id=1),policy TEXT NOT NULL);
PRAGMA user_version=5;
