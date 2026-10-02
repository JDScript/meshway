# SQLite 结构完整目录

冻结源码 `2132434` 的新建临时 store；schema v11。仅结构，没有业务行。

`notnull` 是 PRAGMA 原始标记；是否允许 NULL 还须结合主键、STRICT / WITHOUT ROWID、CHECK 和 trigger 判断。

完整定义见同目录 `schema.sql`，机器数据为 `inventory.json` / `columns.csv`。

## account_invitations

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `invitation_id` | `TEXT` | 0 | 1 | `—` |
| `token_hash` | `BLOB` | 1 | 0 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `email` | `TEXT` | 1 | 0 | `—` |
| `role` | `TEXT` | 1 | 0 | `—` |
| `issuer_user_id` | `TEXT` | 0 | 0 | `—` |
| `expires_at` | `INTEGER` | 1 | 0 | `—` |
| `revoked_at` | `INTEGER` | 0 | 0 | `—` |
| `consumed_by` | `TEXT` | 0 | 0 | `—` |
| `consumed_at` | `INTEGER` | 0 | 0 | `—` |

- FK `consumed_by` → `account_users.user_id`；delete=NO ACTION，update=NO ACTION。
- FK `issuer_user_id` → `account_users.user_id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## account_memberships

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `membership_id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `user_id` | `TEXT` | 1 | 0 | `—` |
| `role` | `TEXT` | 1 | 0 | `—` |
| `revoked_at` | `INTEGER` | 0 | 0 | `—` |

- FK `user_id` → `account_users.user_id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## account_password_resets

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `reset_id` | `TEXT` | 0 | 1 | `—` |
| `token_hash` | `BLOB` | 1 | 0 | `—` |
| `user_id` | `TEXT` | 1 | 0 | `—` |
| `credential_version` | `INTEGER` | 1 | 0 | `—` |
| `expires_at` | `INTEGER` | 1 | 0 | `—` |
| `revoked_at` | `INTEGER` | 0 | 0 | `—` |
| `consumed_at` | `INTEGER` | 0 | 0 | `—` |

- FK `user_id` → `account_users.user_id`；delete=NO ACTION，update=NO ACTION。

## account_sessions

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `token_hash` | `BLOB` | 0 | 1 | `—` |
| `user_id` | `TEXT` | 1 | 0 | `—` |
| `credential_version` | `INTEGER` | 1 | 0 | `—` |
| `expires_at` | `INTEGER` | 1 | 0 | `—` |
| `revoked_at` | `INTEGER` | 0 | 0 | `—` |

- FK `user_id` → `account_users.user_id`；delete=NO ACTION，update=NO ACTION。

## account_users

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `user_id` | `TEXT` | 0 | 1 | `—` |
| `email` | `TEXT` | 1 | 0 | `—` |
| `email_verified` | `INTEGER` | 1 | 0 | `0` |
| `password_phc` | `TEXT` | 1 | 0 | `—` |
| `credential_version` | `INTEGER` | 1 | 0 | `1` |
| `created_at` | `INTEGER` | 1 | 0 | `—` |


## configuration

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `id` | `INTEGER` | 1 | 2 | `—` |
| `revision` | `INTEGER` | 1 | 0 | `—` |
| `ipv4_prefix` | `TEXT` | 1 | 0 | `—` |
| `ipv6_prefix` | `TEXT` | 1 | 0 | `—` |
| `policy` | `TEXT` | 1 | 0 | `—` |
| `source_jsonc` | `TEXT` | 0 | 0 | `—` |

- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## connection_identities

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `node_id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `public_key` | `TEXT` | 1 | 0 | `—` |
| `capabilities` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## connection_receipts

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `node_id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `revision` | `INTEGER` | 1 | 0 | `—` |
| `digest` | `TEXT` | 1 | 0 | `—` |
| `saved_at` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## connection_runtime_leases

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `node_id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `instance_id` | `TEXT` | 1 | 0 | `—` |
| `epoch` | `INTEGER` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## connection_runtime_reports

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `node_id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `sequence` | `INTEGER` | 1 | 0 | `—` |
| `body` | `BLOB` | 1 | 0 | `—` |
| `reported_at` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `connection_runtime_leases.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `connection_runtime_leases.node_id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## connection_source

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `id` | `INTEGER` | 1 | 2 | `—` |
| `configuration` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## device_account_bindings

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `token_id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `user_id` | `TEXT` | 1 | 0 | `—` |
| `membership_id` | `TEXT` | 1 | 0 | `—` |
| `session_digest` | `BLOB` | 1 | 0 | `—` |
| `credential_version` | `INTEGER` | 1 | 0 | `—` |

- FK `user_id` → `account_users.user_id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `account_memberships.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `membership_id` → `account_memberships.membership_id`；delete=NO ACTION，update=NO ACTION。
- FK `user_id` → `account_memberships.user_id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `enrollment_tokens.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `token_id` → `enrollment_tokens.id`；delete=NO ACTION，update=NO ACTION。

## enrollment_tokens

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `token_hash` | `BLOB` | 1 | 0 | `—` |
| `expires_at` | `INTEGER` | 1 | 0 | `—` |
| `used_node_id` | `TEXT` | 0 | 0 | `—` |
| `binding_hash` | `BLOB` | 0 | 0 | `—` |
| `device_binding` | `TEXT` | 0 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `used_node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## latency_admissions

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `source_node_id` | `TEXT` | 1 | 2 | `—` |
| `measurement_id` | `TEXT` | 1 | 3 | `—` |
| `body_digest` | `BLOB` | 1 | 0 | `—` |
| `received_ms` | `INTEGER` | 1 | 0 | `—` |
| `expires_ms` | `INTEGER` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `source_node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## latency_coverage

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `source_node_id` | `TEXT` | 1 | 2 | `—` |
| `capacity_before_ms` | `INTEGER` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `source_node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## latency_runtime_epochs

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `node_id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `epoch` | `INTEGER` | 1 | 0 | `—` |
| `started_ms` | `INTEGER` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## latency_samples

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `row_id` | `INTEGER` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `source_node_id` | `TEXT` | 1 | 0 | `—` |
| `peer_node_id` | `TEXT` | 1 | 0 | `—` |
| `measurement_id` | `TEXT` | 1 | 0 | `—` |
| `runtime_epoch` | `INTEGER` | 1 | 0 | `—` |
| `measured_ms` | `INTEGER` | 1 | 0 | `—` |
| `received_ms` | `INTEGER` | 1 | 0 | `—` |
| `body` | `BLOB` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `peer_node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `source_node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## managed_enrollment_intents

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `token_id` | `TEXT` | 1 | 2 | `—` |
| `settings` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `enrollment_tokens.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `token_id` → `enrollment_tokens.id`；delete=NO ACTION，update=NO ACTION。

## managed_listeners

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `listener_id` | `TEXT` | 1 | 2 | `—` |
| `node_id` | `TEXT` | 1 | 0 | `—` |
| `settings` | `TEXT` | 1 | 0 | `—` |
| `material_version` | `INTEGER` | 1 | 0 | `—` |
| `certificate_sha256` | `TEXT` | 0 | 0 | `—` |
| `material_nonce` | `BLOB` | 0 | 0 | `—` |
| `material_ciphertext` | `BLOB` | 0 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## managed_transport_capabilities

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `node_id` | `TEXT` | 1 | 2 | `—` |
| `vless_streams` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。

## mesh_access_legacy_v5

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `id` | `INTEGER` | 1 | 2 | `—` |
| `policy` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## mesh_access_source

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `id` | `INTEGER` | 1 | 2 | `—` |
| `policy` | `TEXT` | 1 | 0 | `—` |
| `source_jsonc` | `TEXT` | 0 | 0 | `—` |

- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## network_identity

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `id` | `INTEGER` | 1 | 0 | `—` |
| `network_id` | `TEXT` | 0 | 1 | `—` |
| `name` | `TEXT` | 1 | 0 | `—` |
| `dns` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## networks

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 0 | 1 | `—` |
| `created_at` | `TEXT` | 1 | 0 | `—` |
| `initial` | `INTEGER` | 1 | 0 | `—` |


## node_ownership

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `node_id` | `TEXT` | 1 | 2 | `—` |
| `membership_id` | `TEXT` | 0 | 0 | `—` |

- FK `network_id` → `account_memberships.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `membership_id` → `account_memberships.membership_id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。

## node_tags

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `node_id` | `TEXT` | 1 | 2 | `—` |
| `tag` | `TEXT` | 1 | 3 | `—` |

- FK `network_id` → `tag_definitions.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `tag` → `tag_definitions.tag`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `nodes.network_id`；delete=NO ACTION，update=NO ACTION。
- FK `node_id` → `nodes.id`；delete=NO ACTION，update=NO ACTION。
- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## nodes

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `id` | `TEXT` | 0 | 1 | `—` |
| `network_id` | `TEXT` | 1 | 0 | `—` |
| `credential_hash` | `BLOB` | 1 | 0 | `—` |
| `name` | `TEXT` | 1 | 0 | `—` |
| `agent_version` | `TEXT` | 1 | 0 | `—` |
| `registered_at` | `TEXT` | 1 | 0 | `—` |
| `last_seen` | `TEXT` | 1 | 0 | `—` |
| `revoked_at` | `TEXT` | 0 | 0 | `—` |
| `mesh_ipv4` | `TEXT` | 1 | 0 | `—` |
| `mesh_ipv6` | `TEXT` | 1 | 0 | `—` |
| `country` | `TEXT` | 0 | 0 | `—` |
| `acknowledged_revision` | `INTEGER` | 1 | 0 | `0` |
| `acknowledged_digest` | `TEXT` | 0 | 0 | `—` |

- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。

## settings

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `id` | `INTEGER` | 0 | 1 | `—` |
| `admin_hash` | `BLOB` | 1 | 0 | `—` |


## tag_definitions

STRICT=False；WITHOUT ROWID=False。

| 字段 | SQL 类型 | notnull | 主键位置 | 默认 SQL |
| --- | --- | --- | --- | --- |
| `network_id` | `TEXT` | 1 | 1 | `—` |
| `tag` | `TEXT` | 1 | 2 | `—` |
| `description` | `TEXT` | 1 | 0 | `—` |

- FK `network_id` → `networks.network_id`；delete=NO ACTION，update=NO ACTION。
