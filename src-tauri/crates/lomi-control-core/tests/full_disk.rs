//! Opt-in ENOSPC qualification on a newly created, bounded disk image only.
#![cfg(unix)]
use lomi_control_core::{
    atomic_file::{replace, ReplaceError},
    file_trash::{stage, TrashTarget},
    project_files::ProjectDirectory,
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
    println!("Actual ENOSPC on {total}-byte volume after {written} filler bytes; save preserved source; Trash: {trash_outcome}; both recovered after freeing fixture space.");
}
