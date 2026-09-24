//! Read-only WAL preflight before SQLite can recover or overwrite history.
//! Format: https://www.sqlite.org/fileformat2.html#walformat
use super::{Error, Result};
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path};

// The receipt database is capped at 64 MiB with the default 4096-byte pages.
// An oversized journal requires explicit recovery, not an unbounded startup scan.
const MAX_WAL_BYTES: u64 = 128 * 1024 * 1024;

fn word(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}
fn checksum(bytes: &[u8], little: bool, sum: &mut [u32; 2]) {
    for pair in bytes.as_chunks::<8>().0 {
        let value = |part: &[u8]| {
            if little {
                u32::from_le_bytes(part.try_into().unwrap())
            } else {
                word(part)
            }
        };
        sum[0] = sum[0].wrapping_add(value(&pair[..4])).wrapping_add(sum[1]);
        sum[1] = sum[1].wrapping_add(value(&pair[4..])).wrapping_add(sum[0]);
    }
}

pub(super) fn validate(path: &Path) -> Result<()> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_WAL_BYTES {
        return Err(Error::StorageUnavailable);
    }
    if metadata.len() == 0 {
        return Ok(());
    }
    let mut header = [0; 32];
    file.read_exact(&mut header)?;
    let little = match word(&header[..4]) {
        0x377f0682 => true,
        0x377f0683 => false,
        _ => return Err(Error::StorageUnavailable),
    };
    let page_size = word(&header[8..12]);
    if word(&header[4..8]) != 3_007_000
        || !(512..=65536).contains(&page_size)
        || !page_size.is_power_of_two()
    {
        return Err(Error::StorageUnavailable);
    }
    let mut sum = [0; 2];
    checksum(&header[..24], little, &mut sum);
    if sum != [word(&header[24..28]), word(&header[28..32])] {
        return Err(Error::StorageUnavailable);
    }
    let mut remaining = metadata.len() - 32;
    let mut page = vec![0; page_size as usize];
    while remaining > 0 {
        if remaining < 24 {
            return Err(Error::StorageUnavailable);
        }
        let mut frame = [0; 24];
        file.read_exact(&mut frame)?;
        // Checkpoint reuse can leave complete frames from a previous salt.
        // They are not part of the current WAL and must not invalidate it.
        if frame[8..16] != header[16..24] {
            return Ok(());
        }
        if word(&frame[..4]) == 0 || remaining < 24 + u64::from(page_size) {
            return Err(Error::StorageUnavailable);
        }
        file.read_exact(&mut page)?;
        checksum(&frame[..8], little, &mut sum);
        checksum(&page, little, &mut sum);
        if sum != [word(&frame[16..20]), word(&frame[20..24])] {
            return Err(Error::StorageUnavailable);
        }
        remaining -= 24 + u64::from(page_size);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_checkpoint_reuse_preserves_old_salt_tail() {
        let temp = tempfile::tempdir().unwrap();
        let db = rusqlite::Connection::open(temp.path().join("fixture.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE value(data); INSERT INTO value VALUES(zeroblob(16384)); PRAGMA wal_checkpoint(RESTART);").unwrap();
        let path = temp.path().join("fixture.db-wal");
        let old = std::fs::read(&path).unwrap();
        db.execute_batch("UPDATE value SET data=x'01'||zeroblob(16383);")
            .unwrap();
        let new = std::fs::read(&path).unwrap();
        assert_eq!(old.len(), new.len());
        assert_ne!(&old[16..24], &new[16..24]);
        let last_frame = new.len() - (24 + word(&new[8..12]) as usize);
        assert_eq!(&new[last_frame + 8..last_frame + 16], &old[16..24]);
        validate(&path).unwrap();
    }

    #[test]
    fn truncated_and_oversized_wal_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fixture.db-wal");
        std::fs::write(&path, b"partial").unwrap();
        assert_eq!(validate(&path), Err(Error::StorageUnavailable));
        assert_eq!(std::fs::read(&path).unwrap(), b"partial");
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_WAL_BYTES + 1).unwrap();
        assert_eq!(validate(&path), Err(Error::StorageUnavailable));
        assert_eq!(file.metadata().unwrap().len(), MAX_WAL_BYTES + 1);
    }
}
