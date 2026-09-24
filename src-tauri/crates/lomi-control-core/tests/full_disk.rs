//! Opt-in ENOSPC qualification on a newly created, bounded disk image only.
#![cfg(unix)]
use lomi_control_core::{
    atomic_file::{replace, ReplaceError},
    file_trash::{stage, TrashTarget},
    project_files::ProjectDirectory,
    receipts::{Effect, Error as ReceiptError, Key, State, Store},
};
use lomi_control_protocol::{files::FileEntryKind, ErrorCode};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{self, File, Permissions},
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    },
    path::PathBuf,
};

#[test]
#[ignore = "Run tests/native/run-mcp-full-disk.mjs on its isolated 32 MiB volume"]
fn full_disk_preserves_atomic_save_and_trash_source_then_recovers() {
    let volume = PathBuf::from(
        std::env::var_os("LOMI_MCP_FULL_DISK_DIRECTORY")
            .expect("An isolated test volume is required"),
    )
    .canonicalize()
    .unwrap();
    assert!(volume.join(".lomi-mcp-full-disk").is_file());
    assert_ne!(
        fs::metadata(&volume).unwrap().dev(),
        fs::metadata(volume.parent().unwrap()).unwrap().dev(),
        "Refusing to fill the host filesystem"
    );
    let name = CString::new(volume.as_os_str().as_bytes()).unwrap();
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    assert_eq!(
        unsafe { libc::statvfs(name.as_ptr(), info.as_mut_ptr()) },
        0
    );
    let info = unsafe { info.assume_init() };
    let total = u128::from(info.f_blocks) * u128::from(info.f_frsize);
    assert!(
        (8 * 1024 * 1024..=64 * 1024 * 1024).contains(&total),
        "Only a small fixture volume may be filled"
    );
    let project = volume.join("project");
    let recovery = volume.join("recovery");
    fs::create_dir(&project).unwrap();
    fs::create_dir(&recovery).unwrap();
    fs::set_permissions(&recovery, Permissions::from_mode(0o700)).unwrap();
    let path = project.join("original.txt");
    fs::write(&path, b"original bytes must survive").unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    let hash = format!("{:x}", Sha256::digest(b"original bytes must survive"));
    let directory = ProjectDirectory::open(&project).unwrap();
    let control = volume.join("control");
    let mut store = Store::open(&control, 100).unwrap();
    let epoch = store.issue_epoch("fixture-client", 100).unwrap();
    fn key<'a>(epoch: &'a str, request_key: &'a str) -> Key<'a> {
        Key {
            pairing_id: "fixture-client",
            retry_epoch: epoch,
            project_id: "fixture-project",
            tool: "fixture-mutation",
            request_key,
        }
    }
    let original = store
        .reserve(&key(&epoch, "original"), [1; 32], 101)
        .unwrap();
    store
        .transition(
            "fixture-client",
            "fixture-project",
            &original.receipt.operation_id,
            State::Running,
            Effect::None,
            102,
        )
        .unwrap();
    let mut filler = File::create_new(volume.join("owned-filler")).unwrap();
    let block = vec![0x53_u8; 65536];
    let mut written = 0_u64;
    let mut errors = 0;
    for size in [65536, 4096, 512, 1] {
        loop {
            match filler.write(&block[..size]) {
                Ok(n) => {
                    assert!(n > 0);
                    written += n as u64;
                    assert!(u128::from(written) <= total);
                }
                Err(error) => {
                    assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
                    errors += 1;
                    break;
                }
            }
        }
    }
    if let Err(error) = filler.sync_all() {
        assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
    }
    assert_eq!(errors, 4);
    // APFS can retain some metadata capacity after a file write gets ENOSPC.
    // Bound the attempts, and require an actual failed durable reservation.
    let mut failed_key = None;
    let mut accepted = Vec::new();
    for index in 0..256 {
        let request_key = format!("full-{index}");
        match store.reserve(&key(&epoch, &request_key), [2; 32], 103) {
            Ok(reservation) => accepted.push(reservation.receipt.operation_id),
            Err(ReceiptError::StorageUnavailable) => {
                failed_key = Some(request_key);
                break;
            }
            Err(error) => panic!("Unexpected receipt error on full volume: {error:?}"),
        }
    }
    let failed_key =
        failed_key.expect("SQLite must reject a reservation on the full fixture volume");
    let replacement = vec![0x52; 4 * 1024 * 1024];
    let save = replace(&path, &hash, &replacement, || Ok(()));
    assert!(
        matches!(
            save,
            Err(ReplaceError::Before(ErrorCode::StorageUnavailable))
        ),
        "{save:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), b"original bytes must survive");
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::read_dir(&project).unwrap().count(),
        1,
        "Failed save leaked its temporary file"
    );
    let parent = directory.list("", || Ok(())).unwrap().revision;
    let trash = stage(
        &directory,
        TrashTarget {
            relative_path: "original.txt",
            kind: &FileEntryKind::File,
            expected_revision: &hash,
            expected_parent_revision: &parent,
        },
        &recovery,
        "full-disk-trash",
        || Ok(()),
    );
    let (preserved, trash_outcome) = match trash {
        Err(ReplaceError::Before(ErrorCode::StorageUnavailable)) => (
            path.clone(),
            "plan could not be persisted; source unchanged",
        ),
        Ok(staged) => {
            // APFS may reclaim or reserve enough metadata space for a rename
            // after a large data write receives ENOSPC. A successful stage is
            // legitimate; a failed OS handoff must preserve the journal+inode.
            let preserved = staged.path().to_path_buf();
            let plan: serde_json::Value = serde_json::from_slice(
                &fs::read(recovery.join("full-disk-trash/plan.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(plan["expectedRevision"], hash);
            assert_eq!(plan["inode"], inode.to_string());
            assert!(matches!(
                staged.finish(|_| Err(ErrorCode::StorageUnavailable)),
                Err(ReplaceError::Uncertain)
            ));
            assert!(!path.exists());
            (
                preserved,
                "metadata staging succeeded; rejected OS handoff retained exact recovery bytes",
            )
        }
        Err(error) => panic!("Unexpected full-volume Trash result: {error:?}"),
    };
    assert_eq!(
        fs::read(&preserved).unwrap(),
        b"original bytes must survive"
    );
    assert_eq!(fs::metadata(&preserved).unwrap().ino(), inode);
    drop(filler);
    fs::remove_file(volume.join("owned-filler")).unwrap();
    assert!(
        store
            .existing(&key(&epoch, &failed_key), 104)
            .unwrap()
            .is_none(),
        "A failed reservation must not publish a partial receipt"
    );
    drop(store);
    let mut store = Store::open(&control, 105).unwrap();
    for id in std::iter::once(&original.receipt.operation_id).chain(&accepted) {
        let receipt = store.get("fixture-client", "fixture-project", id).unwrap();
        assert_eq!(receipt.state, State::OutcomeUnknown);
        assert_eq!(receipt.effect_state, Effect::Unknown);
    }
    assert_eq!(
        store
            .reserve(&key(&epoch, &failed_key), [2; 32], 106)
            .unwrap_err(),
        ReceiptError::RetryWindowExpired
    );
    let fresh_epoch = store.issue_epoch("fixture-client", 106).unwrap();
    let fresh_key = Key {
        retry_epoch: &fresh_epoch,
        ..key(&epoch, &failed_key)
    };
    assert!(store.reserve(&fresh_key, [2; 32], 107).unwrap().created);
    if preserved != path {
        // Explicit recovery of this test-owned file only; production never
        // automatically puts back an unresolved Trash handoff.
        fs::rename(&preserved, &path).unwrap();
    }
    let hash = replace(&path, &hash, &replacement, || Ok(())).unwrap();
    assert_eq!(fs::read(&path).unwrap(), replacement);
    let parent = directory.list("", || Ok(())).unwrap().revision;
    let staged = stage(
        &directory,
        TrashTarget {
            relative_path: "original.txt",
            kind: &FileEntryKind::File,
            expected_revision: &hash,
            expected_parent_revision: &parent,
        },
        &recovery,
        "recovered-trash",
        || Ok(()),
    )
    .unwrap();
    let destination = volume.join("fixture-trash");
    staged
        .finish(|source| {
            fs::rename(source, &destination).map_err(|_| ErrorCode::StorageUnavailable)
        })
        .unwrap();
    assert_eq!(fs::read(&destination).unwrap(), replacement);
    assert!(!path.exists());
    println!("Actual ENOSPC on {total}-byte volume after {written} filler bytes; SQLite rejected {failed_key} after {} additional durable reservations, retained receipts recovered without replay, old epoch rejected; save preserved source; Trash: {trash_outcome}; all recovered after freeing fixture space.", accepted.len());
}
