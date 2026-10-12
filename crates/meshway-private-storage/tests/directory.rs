#![cfg(unix)]
use meshway_private_storage::{Directory, Error};
use std::{
    fs,
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, Barrier},
};

#[test]
fn existing_owner_inspection_never_creates_paths_or_replaces_a_missing_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let absent = root.join("absent").join("owner");
    assert!(
        Directory::open_existing(&absent, "owner.lock")
            .unwrap()
            .is_none()
    );
    assert!(!root.join("absent").exists());
    let path = root.join("private");
    let owner = Directory::open(&path, "owner.lock").unwrap();
    assert!(matches!(
        Directory::open_existing(&path, "owner.lock"),
        Err(Error::InUse)
    ));
    drop(owner);
    let inspected = Directory::open_existing(&path, "owner.lock")
        .unwrap()
        .unwrap();
    assert_eq!(fs::read_dir(&path).unwrap().count(), 1);
    drop(inspected);
    fs::remove_file(path.join("owner.lock")).unwrap();
    assert!(matches!(
        Directory::open_existing(&path, "owner.lock"),
        Err(Error::UnsafeStorage)
    ));
    assert!(!path.join("owner.lock").exists());
}

#[test]
fn exclusive_lock_and_atomic_replacements_keep_whole_private_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("private");
    let directory = Arc::new(Directory::open(&path, "owner.lock").unwrap());
    assert!(matches!(
        Directory::open(&path, "owner.lock"),
        Err(Error::InUse)
    ));
    let old = vec![b'a'; 4096];
    let new = vec![b'b'; 4096];
    directory.replace("state", &old).unwrap();
    assert!(directory.replace("owner.lock", &old).is_err());
    let barrier = Arc::new(Barrier::new(2));
    let reader = {
        let d = directory.clone();
        let b = barrier.clone();
        std::thread::spawn(move || {
            b.wait();
            for _ in 0..100 {
                let mut bytes = Vec::new();
                d.file("state", false)
                    .unwrap()
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                assert!(bytes == vec![b'a'; 4096] || bytes == vec![b'b'; 4096]);
            }
        })
    };
    barrier.wait();
    for n in 0..20 {
        directory
            .replace("state", if n % 2 == 0 { &new } else { &old })
            .unwrap();
    }
    reader.join().unwrap();
    assert_eq!(
        fs::metadata(path.join("state"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read_dir(&path).unwrap().count(), 2);
    drop(directory);
    Directory::open(&path, "owner.lock").unwrap();
}

#[test]
fn publication_never_overwrites_and_unsafe_destinations_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("private");
    let directory = Directory::open(&path, "owner.lock").unwrap();
    directory
        .publish_once("key", "key.pending", Some(b"original"))
        .unwrap();
    assert!(
        directory
            .publish_once("key", "key.pending", Some(b"replacement"))
            .is_err()
    );
    assert_eq!(fs::read(path.join("key")).unwrap(), b"original");
    assert_eq!(fs::read(path.join("key.pending")).unwrap(), b"replacement");
    symlink(path.join("key"), path.join("link")).unwrap();
    assert!(matches!(
        directory.replace("link", b"no"),
        Err(Error::UnsafeStorage)
    ));
    fs::hard_link(path.join("key"), path.join("hardlink")).unwrap();
    assert!(matches!(
        directory.replace("key", b"no"),
        Err(Error::UnsafeStorage)
    ));
    assert!(directory.file("../escape", true).is_err());
    assert!(directory.file("a/b", true).is_err());
    assert!(directory.file(".", true).is_err());
}

#[test]
fn stale_archives_and_staging_files_are_recognized_by_their_exact_names() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("private");
    let directory = Directory::open(&path, "owner.lock").unwrap();
    directory.replace("journal.json", b"{}").unwrap();
    let archive = directory.archive_stale("journal.json", 3).unwrap();
    assert!(meshway_private_storage::stale_stamp(&archive, "journal.json").is_some());
    assert!(meshway_private_storage::stale_stamp(&archive, "other.json").is_none());
    assert!(meshway_private_storage::stale_stamp("journal.json.stale-", "journal.json").is_none());
    assert!(
        meshway_private_storage::stale_stamp("journal.json.stale-1x", "journal.json").is_none()
    );
    assert!(meshway_private_storage::stale_stamp("journal.json", "journal.json").is_none());
    let names: Vec<String> = fs::read_dir(&path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&archive));
    assert!(
        !names
            .iter()
            .any(|name| meshway_private_storage::is_staging(name))
    );
    assert!(meshway_private_storage::is_staging(
        ".write-0123456789abcdef0123456789abcdef"
    ));
    assert!(!meshway_private_storage::is_staging(".write-0123"));
    assert!(!meshway_private_storage::is_staging(
        "write-0123456789abcdef0123456789abcdef"
    ));
}
