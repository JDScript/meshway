use super::*;
use std::{cell::RefCell, io::Read, os::unix::fs::PermissionsExt, sync::Arc};

thread_local! {
    static AFTER_OPEN: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

pub(super) fn after_open() {
    let hook = AFTER_OPEN.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

fn once(hook: impl FnOnce() + 'static) {
    AFTER_OPEN.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
}

#[test]
fn opened_old_inode_is_reopened_after_real_atomic_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        Directory::open(&temp.path().canonicalize().unwrap().join("owner"), "lock").unwrap(),
    );
    directory.replace("state", b"old").unwrap();
    let writer = directory.clone();
    once(move || writer.replace("state", b"new").unwrap());
    let mut bytes = Vec::new();
    directory
        .file("state", false)
        .unwrap()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"new");
}

#[test]
fn reopening_never_accepts_an_unlinked_or_unsafe_replacement() {
    for kind in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let directory = Arc::new(
            Directory::open(&temp.path().canonicalize().unwrap().join("owner"), "lock").unwrap(),
        );
        directory.replace("state", b"old").unwrap();
        let writer = directory.clone();
        once(move || {
            let path = writer.path().join("state");
            std::fs::remove_file(&path).unwrap();
            match kind {
                0 => {}
                1 => {
                    writer.replace("other", b"hardlink").unwrap();
                    std::fs::hard_link(writer.path().join("other"), path).unwrap();
                }
                2 => std::os::unix::fs::symlink("missing", path).unwrap(),
                3 => {
                    std::fs::write(&path, b"public").unwrap();
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
                }
                _ => unreachable!(),
            }
        });
        // create=true must not recreate a name that vanished after our open.
        assert!(matches!(
            directory.file("state", true),
            Err(Error::UnsafeStorage)
        ));
    }
}

#[test]
fn continuous_replacement_is_bounded() {
    fn replace_again(directory: Arc<Directory>) {
        once(move || {
            directory.replace("state", b"new").unwrap();
            replace_again(directory);
        });
    }
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        Directory::open(&temp.path().canonicalize().unwrap().join("owner"), "lock").unwrap(),
    );
    directory.replace("state", b"old").unwrap();
    replace_again(directory.clone());
    assert!(matches!(
        directory.file("state", false),
        Err(Error::UnsafeStorage)
    ));
    AFTER_OPEN.with(|slot| {
        slot.borrow_mut().take();
    });
    assert!(directory.file("state", false).unwrap().is_some());
}

#[test]
fn stale_archives_keep_only_the_newest() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap().join("owner");
    let directory = Directory::open(&path, "owner.lock").unwrap();
    let mut archives = Vec::new();
    for round in 0..5u8 {
        directory.replace("journal.json", &[round]).unwrap();
        archives.push(directory.archive_stale("journal.json", 3).unwrap());
        assert!(directory.file("journal.json", false).unwrap().is_none());
    }
    directory.replace("other.json.stale-1", b"x").unwrap();
    let mut names: Vec<_> = std::fs::read_dir(&path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("journal.json.stale-"))
        .collect();
    names.sort();
    assert_eq!(names, archives[2..]);
    for (round, name) in archives[2..].iter().enumerate() {
        assert_eq!(std::fs::read(path.join(name)).unwrap(), [round as u8 + 2]);
    }
    assert!(path.join("other.json.stale-1").exists());
}
