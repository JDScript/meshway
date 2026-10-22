//! 8 → 9: the `schema_migrations` audit table; every other table is copied.
use super::Level;

pub(super) const LEVEL: Level = Level {
    to: 9,
    new_tables: &["schema_migrations"],
    dropped_tables: &[],
    transforms: &[],
};
