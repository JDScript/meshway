//! The Server's netmap signing keys: sealed rows, rotation and pinning status.
use super::*;

/// One stored netmap signing key: its public binding, when it was created
/// (the `notBefore` of the handover to it) and its sealed private seed.
pub(crate) struct SigningKeyRow {
    pub(crate) public: meshway_protocol::netmap::ServerSigningKey,
    pub(crate) created_at: i64,
    sealed: Vec<u8>,
}

impl SigningKeyRow {
    /// Opens the sealed seed; it must derive the stored public key.
    pub(crate) fn seed(
        &self,
        sealer: &Sealer,
        public_of: impl Fn(&[u8; 32]) -> [u8; 32],
    ) -> Result<zeroize::Zeroizing<[u8; 32]>, Error> {
        let plaintext = sealer.open_signing_key(
            self.public.key_id,
            self.public.key_generation,
            &self.sealed,
        )?;
        let mut seed = zeroize::Zeroizing::new([0; 32]);
        if plaintext.as_bytes().len() != 32 {
            return Err(Error::decode("server_signing_keys.private_key"));
        }
        seed.copy_from_slice(plaintext.as_bytes());
        if &public_of(&seed) != self.public.public_key.bytes.as_bytes() {
            return Err(Error::decode("server_signing_keys.private_key"));
        }
        Ok(seed)
    }
}

/// The keys that sign sync answers at one instant: the current key and,
/// while its rotation window is open, the key it replaces.
pub(crate) struct SigningKeys {
    pub(crate) current: SigningKeyRow,
    /// The previous generation whose `retired_at` (the end of its rotation
    /// window) is still in the future.
    pub(crate) rotating: Option<(SigningKeyRow, i64)>,
}

type KeyColumns = (Vec<u8>, i64, Vec<u8>, Vec<u8>, i64, Option<i64>);

fn key_row(
    (key_id, generation, public, sealed, created_at, _): KeyColumns,
) -> Result<SigningKeyRow, Error> {
    Ok(SigningKeyRow {
        public: public_signing_key(
            id(key_id)?,
            u64::try_from(generation)
                .ok()
                .and_then(|value| Version::new(value).ok())
                .ok_or(Error::decode("server_signing_keys.generation"))?,
            public
                .try_into()
                .map_err(|_| Error::decode("server_signing_keys.public_key"))?,
        ),
        created_at,
        sealed,
    })
}

const KEY_COLUMNS: &str =
    "SELECT id,generation,public_key,private_key,created_at,retired_at FROM server_signing_keys";

fn key_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<KeyColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

/// The signing keys in effect at `now`; an internal fault before the first key exists.
pub(crate) fn signing_keys(tx: &Transaction<'_>, now: i64) -> Result<SigningKeys, Error> {
    let current = current_key(tx)?;
    let rotating = tx
        .query_row(
            &format!("{KEY_COLUMNS} WHERE purpose='netmap' AND generation=?1 AND retired_at>?2"),
            params![current.public.key_generation.get() as i64 - 1, now],
            key_columns,
        )
        .optional()?
        .map(|columns| {
            let retires_at = columns
                .5
                .ok_or(Error::decode("server_signing_keys.retired_at"))?;
            key_row(columns).map(|row| (row, retires_at))
        })
        .transpose()?;
    Ok(SigningKeys { current, rotating })
}

fn current_key(tx: &Transaction<'_>) -> Result<SigningKeyRow, Error> {
    key_row(
        tx.query_row(
            &format!("{KEY_COLUMNS} WHERE purpose='netmap' AND retired_at IS NULL"),
            [],
            key_columns,
        )
        .optional()?
        .ok_or(Error::Internal("netmap signing key missing"))?,
    )
}

/// Creates generation 1 when no netmap signing key exists yet and checks that
/// the keys in effect open and match their public halves. The private seed
/// never leaves the sealed column in plaintext.
pub(crate) fn ensure_signing_key(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    now: i64,
    generate: impl FnOnce() -> Result<zeroize::Zeroizing<[u8; 32]>, Error>,
    public_of: impl Fn(&[u8; 32]) -> [u8; 32],
) -> Result<(), Error> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM server_signing_keys WHERE purpose='netmap')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        insert_signing_key(tx, sealer, Version::MIN, now, generate()?, &public_of)?;
    }
    let keys = signing_keys(tx, now)?;
    keys.current.seed(sealer, &public_of)?;
    if let Some((rotating, _)) = &keys.rotating {
        rotating.seed(sealer, &public_of)?;
    }
    Ok(())
}

fn insert_signing_key(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    generation: Version,
    now: i64,
    seed: zeroize::Zeroizing<[u8; 32]>,
    public_of: impl Fn(&[u8; 32]) -> [u8; 32],
) -> Result<meshway_protocol::netmap::ServerSigningKey, Error> {
    let key_id = crate::secrets::random_key_id()?;
    let public = public_of(&seed);
    let sealed = sealer.seal_signing_key(key_id, generation, seed.as_slice())?;
    tx.execute(
        "INSERT INTO server_signing_keys(id,purpose,generation,public_key,private_key,created_at)
         VALUES(?1,'netmap',?2,?3,?4,?5)",
        params![
            bytes(key_id.as_uuid()),
            generation.get() as i64,
            public.as_slice(),
            sealed.as_bytes(),
            now
        ],
    )?;
    Ok(public_signing_key(key_id, generation, public))
}

/// One committed rotation of the netmap signing key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotatedSigningKey {
    /// The key that endorses its successor until the window ends.
    pub previous: meshway_protocol::netmap::ServerSigningKey,
    /// The new current key; Nodes enrolled from now on pin it directly.
    pub next: meshway_protocol::netmap::ServerSigningKey,
    /// When the handover window ends, in Unix milliseconds.
    pub window_ends_at_ms: u64,
}

/// Starts a rotation: a new key of the next generation becomes current at
/// `now` (its `notBefore`) and the replaced key's `retired_at` is set to the
/// end of the window. Only one window may be open at a time.
pub(crate) fn rotate_signing_key(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    now: i64,
    window_ms: i64,
    generate: impl FnOnce() -> Result<zeroize::Zeroizing<[u8; 32]>, Error>,
    public_of: impl Fn(&[u8; 32]) -> [u8; 32],
) -> Result<RotatedSigningKey, Error> {
    let keys = signing_keys(tx, now)?;
    if keys.rotating.is_some() {
        return Err(Error::StateConflict);
    }
    let previous = keys.current.public;
    let generation = previous
        .key_generation
        .checked_next()
        .map_err(|_| Error::Internal("netmap signing key generation overflow"))?;
    let ends = now.checked_add(window_ms).ok_or(Error::InvalidRequest)?;
    tx.execute(
        "UPDATE server_signing_keys SET retired_at=?2 WHERE id=?1 AND retired_at IS NULL",
        params![bytes(previous.key_id.as_uuid()), ends],
    )?;
    let next = insert_signing_key(tx, sealer, generation, now, generate()?, public_of)?;
    Ok(RotatedSigningKey {
        previous,
        next,
        window_ends_at_ms: u64::try_from(ends).map_err(|_| Error::InvalidRequest)?,
    })
}

/// How a Node's pinned key generation relates to the keys in effect at `now`.
pub(crate) fn pinned_status(
    tx: &Transaction<'_>,
    now: i64,
    generation: Version,
) -> Result<meshway_protocol::PinnedServerKeyStatus, Error> {
    use meshway_protocol::PinnedServerKeyStatus as Status;
    let keys = signing_keys(tx, now)?;
    Ok(if generation == keys.current.public.key_generation {
        Status::Current
    } else if keys
        .rotating
        .is_some_and(|(row, _)| row.public.key_generation == generation)
    {
        Status::Rotating
    } else {
        Status::KeyMismatch
    })
}

/// Public half of the current netmap signing key, for enrollment responses.
pub(crate) fn server_signing_key(
    tx: &Transaction<'_>,
) -> Result<meshway_protocol::netmap::ServerSigningKey, Error> {
    Ok(current_key(tx)?.public)
}

fn public_signing_key(
    key_id: meshway_model::KeyId,
    key_generation: Version,
    public: [u8; 32],
) -> meshway_protocol::netmap::ServerSigningKey {
    meshway_protocol::netmap::ServerSigningKey {
        key_id,
        key_generation,
        public_key: meshway_model::NodePublicKey {
            algorithm: meshway_model::KeyAlgorithm::Ed25519,
            bytes: Digest::from_bytes(public),
        },
    }
}
