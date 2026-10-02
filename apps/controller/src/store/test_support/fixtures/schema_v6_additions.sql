ALTER TABLE configuration ADD COLUMN source_jsonc TEXT;
ALTER TABLE mesh_access_source RENAME TO mesh_access_legacy_v5;
CREATE TABLE mesh_access_source(id INTEGER PRIMARY KEY CHECK(id=1),policy TEXT NOT NULL,source_jsonc TEXT);
CREATE TABLE tag_definitions(tag TEXT PRIMARY KEY,description TEXT NOT NULL);
CREATE TABLE node_tags(node_id TEXT NOT NULL REFERENCES nodes(id),tag TEXT NOT NULL REFERENCES tag_definitions(tag),PRIMARY KEY(node_id,tag));
PRAGMA user_version=6;
