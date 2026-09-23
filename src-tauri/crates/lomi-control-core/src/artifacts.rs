//! Immutable, source-classified blobs. The receipt store's owner lock and SQLite
//! transaction serialize reservations; a blob is never readable before fsync.
use crate::receipts::{private, Error, Store};
use lomi_control_protocol::{
    artifact::{
        AndroidImageGeometry, Artifact, ArtifactSource, BrowserArtifactSource,
        BrowserImageGeometry, ImageGeometry,
    },
    control::valid_id,
};
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
};

type Result<T> = std::result::Result<T, Error>;
pub const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;
const MAX_IMAGE_TOTAL_BYTES: i64 = 64 * 1024 * 1024;
const MAX_IMAGE_OWNER_BYTES: i64 = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: i64 = 2 * 1024 * 1024 * 1024;
const MAX_OWNER_BYTES: i64 = 1024 * 1024 * 1024;
const RETENTION: i64 = 24 * 60 * 60;
pub struct Reservation {
    pub id: String,
    pub max_bytes: usize,
    owner: String,
    project: String,
    class: &'static str,
    _pin: std::sync::Arc<()>,
}
pub struct ArtifactFile {
    pub file: fs::File,
    pub artifact: Artifact,
    _pin: std::sync::Arc<()>,
}
impl ArtifactFile {
    /// Read and hash in bounded chunks outside the store lock. Rewind the exact
    /// descriptor for its consumer; never reopen the project source pathname.
    pub fn verify(
        &mut self,
        check: impl Fn() -> std::result::Result<(), lomi_control_protocol::ErrorCode>,
    ) -> std::result::Result<(), lomi_control_protocol::ErrorCode> {
        use lomi_control_protocol::ErrorCode;
        use std::io::{Seek, SeekFrom};
        let failure = |_| ErrorCode::StorageUnavailable;
        self.file.seek(SeekFrom::Start(0)).map_err(failure)?;
        let mut hash = Sha256::new();
        let mut remaining = u64::from(self.artifact.byte_length);
        let mut buffer = [0; 65536];
        while remaining > 0 {
            check()?;
            let limit = remaining.min(buffer.len() as u64) as usize;
            let count = self.file.read(&mut buffer[..limit]).map_err(failure)?;
            if count == 0 {
                return Err(ErrorCode::StorageUnavailable);
            }
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        if self.file.read(&mut buffer[..1]).map_err(failure)? != 0
            || format!("{:x}", hash.finalize()) != self.artifact.sha256
        {
            return Err(ErrorCode::StorageUnavailable);
        }
        self.file.seek(SeekFrom::Start(0)).map_err(failure)?;
        check()
    }
}
fn blob_id(id: &str) -> Result<()> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn source_valid(source: &ArtifactSource) -> bool {
    match source {
        ArtifactSource::Browser(source) => browser_source_valid(source),
        ArtifactSource::Project(s) => {
            valid_id(&s.workspace_id)
                && s.required_scope == "files.read"
                && crate::project_files::validate_relative(&s.relative_path).is_ok()
                && s.relative_path.ends_with(".apk")
        }
        ArtifactSource::Android(s) => {
            valid_id(&s.workspace_id)
                && valid_id(&s.panel_id)
                && lomi_control_protocol::android::valid_device_id(&s.device_id)
                && lomi_control_protocol::android::valid_device_id(&s.generation)
                && s.required_scope == "android.capture"
        }
    }
}
fn browser_source_valid(source: &BrowserArtifactSource) -> bool {
    [
        &source.workspace_id,
        &source.panel_id,
        &source.browser_generation,
        &source.profile_id,
        &source.navigation_id,
    ]
    .into_iter()
    .all(|s| valid_id(s))
        && source.required_scope == "browser.capture_composite"
        && lomi_control_protocol::browser::Origin::parse(&source.origin)
            .is_ok_and(|o| o.as_str() == source.origin)
}
fn geometry_valid(g: &ImageGeometry) -> bool {
    match g {
        ImageGeometry::Browser(g) => browser_geometry_valid(g),
        ImageGeometry::Android(g) => {
            let (w, h) = if g.rotation % 2 == 0 {
                (g.hardware_display[0], g.hardware_display[1])
            } else {
                (g.hardware_display[1], g.hardware_display[0])
            };
            g.captured_at_millis.parse::<u64>().is_ok()
                && g.rotation <= 3
                && g.hardware_display.iter().all(|n| (1..=4096).contains(n))
                && u64::from(w) * u64::from(h) <= 8_294_400
                && g.pixel_width > 0
                && g.pixel_height > 0
                && g.pixel_width <= 1600
                && g.pixel_height <= 1600
                && u64::from(g.pixel_width) * u64::from(g.pixel_height) <= 2_000_000
                && (g.capture_scale_x - f64::from(g.pixel_width) / f64::from(w)).abs() < 1e-9
                && (g.capture_scale_y - f64::from(g.pixel_height) / f64::from(h)).abs() < 1e-9
                && g.image_to_hardware.iter().all(|n| n.is_finite())
                && g.image_to_hardware
                    == AndroidImageGeometry::transform(
                        g.hardware_display,
                        g.pixel_width,
                        g.pixel_height,
                        g.rotation,
                    )
                && g.coordinate_space == "image_pixels"
                && g.crop == "full_display"
        }
    }
}
fn classification_matches(source: &ArtifactSource, image: &ImageGeometry) -> bool {
    matches!(
        (source, image),
        (ArtifactSource::Browser(_), ImageGeometry::Browser(_))
            | (ArtifactSource::Android(_), ImageGeometry::Android(_))
    )
}
fn browser_geometry_valid(g: &BrowserImageGeometry) -> bool {
    g.captured_at_millis.parse::<u64>().is_ok()
        && [
            g.css_width,
            g.css_height,
            g.device_scale_factor,
            g.capture_scale_x,
            g.capture_scale_y,
            g.page_zoom,
            g.scroll_x,
            g.scroll_y,
        ]
        .into_iter()
        .all(f64::is_finite)
        && g.css_width > 0.
        && g.css_width <= 100000.
        && g.css_height > 0.
        && g.css_height <= 100000.
        && g.device_scale_factor > 0.
        && g.device_scale_factor <= 16.
        && g.page_zoom > 0.
        && g.page_zoom <= 16.
        && g.pixel_width > 0
        && g.pixel_height > 0
        && g.pixel_width <= 4096
        && g.pixel_height <= 4096
        && u64::from(g.pixel_width) * u64::from(g.pixel_height) <= 2_000_000
        && (g.capture_scale_x - f64::from(g.pixel_width) / g.css_width).abs() < 1e-9
        && (g.capture_scale_y - f64::from(g.pixel_height) / g.css_height).abs() < 1e-9
        && g.crop == "viewport"
}
impl Store {
    fn blob_path(&self, id: &str) -> Result<std::path::PathBuf> {
        blob_id(id)?;
        let class: String = self
            .connection
            .query_row("SELECT class FROM artifacts WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or(Error::TargetNotFound)?;
        let extension = match class.as_str() {
            "image" => "png",
            "apk" => "apk",
            _ => return Err(Error::StorageUnavailable),
        };
        Ok(self
            .root
            .join("artifacts")
            .join(format!("{id}.{extension}")))
    }
    /// At most 64 previously recorded paths. No recursive scan or arbitrary cleanup.
    pub(crate) fn recover_artifacts(&mut self, now: i64) -> Result<()> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM artifacts", [], |r| r.get(0))?;
        if count > 64 {
            return Err(Error::StorageUnavailable);
        }
        let obsolete = self
            .connection
            .prepare("SELECT id FROM artifacts WHERE state='reserved' OR expires <= ?1")?
            .query_map([now], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in obsolete {
            if !self.artifact_pinned(&id) {
                self.remove_artifact(&id)?;
            }
        }
        Ok(())
    }
    fn remove_artifact(&mut self, id: &str) -> Result<()> {
        let path = self.blob_path(id)?;
        let directory = path.parent().unwrap();
        match directory.symlink_metadata() {
            Ok(_) => private(directory, true)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        // The durable reservation owns all three names, including a crash in
        // between hard-link publication and unlinking the temporary name.
        for path in [
            directory.join(format!("{id}.stage")),
            directory.join(format!("{id}.ready")),
            path.clone(),
        ] {
            match path.symlink_metadata() {
                Ok(m) => {
                    use std::os::unix::fs::MetadataExt;
                    if !m.is_file()
                        || m.uid() != unsafe { libc::geteuid() }
                        || m.mode() & 0o777 != 0o600
                        || !(1..=3).contains(&m.nlink())
                    {
                        return Err(Error::StorageUnavailable);
                    }
                    fs::remove_file(&path)?;
                    fs::File::open(directory)?.sync_all()?;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.connection
            .execute("DELETE FROM artifacts WHERE id=?1", [id])?;
        self.artifact_pins.remove(id);
        Ok(())
    }
    fn artifact_pinned(&self, id: &str) -> bool {
        self.artifact_pins
            .get(id)
            .is_some_and(|pin| pin.strong_count() > 0)
    }
    pub fn reserve_artifact(
        &mut self,
        owner: &str,
        project: &str,
        source: &ArtifactSource,
        max_bytes: usize,
        now: i64,
    ) -> Result<Reservation> {
        let class = if matches!(source, ArtifactSource::Project(_)) {
            "apk"
        } else {
            "image"
        };
        let maximum = if class == "apk" {
            crate::staging::MAX_IMPORT_BYTES as usize
        } else {
            MAX_IMAGE_BYTES
        };
        if !valid_id(owner)
            || !valid_id(project)
            || !source_valid(source)
            || !(1..=maximum).contains(&max_bytes)
            || now < 0
        {
            return Err(Error::InvalidInput);
        }
        let expires = now.checked_add(RETENTION).ok_or(Error::InvalidInput)?;
        // Only expired rows are reclaimed while producers may still be running.
        let expired = self
            .connection
            .prepare("SELECT id FROM artifacts WHERE expires <= ?1 LIMIT 64")?
            .query_map([now], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in expired {
            if !self.artifact_pinned(&id) {
                self.remove_artifact(&id)?;
            }
        }
        let dir = self.root.join("artifacts");
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => fs::File::open(&self.root)?.sync_all()?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        private(&dir, true)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (count,bytes,running):(i64,i64,i64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(reserved_bytes),0),COALESCE(SUM(state='reserved'),0) FROM artifacts",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let owned: i64 = tx.query_row(
            "SELECT COALESCE(SUM(reserved_bytes),0) FROM artifacts WHERE pairing=?1",
            [owner],
            |r| r.get(0),
        )?;
        let project_bytes: i64 = tx.query_row(
            "SELECT COALESCE(SUM(reserved_bytes),0) FROM artifacts WHERE project=?1",
            [project],
            |r| r.get(0),
        )?;
        let (image_bytes, owner_images): (i64, i64) = tx.query_row("SELECT COALESCE(SUM(reserved_bytes),0),COALESCE(SUM(CASE WHEN pairing=?1 THEN reserved_bytes ELSE 0 END),0) FROM artifacts WHERE class='image'", [owner], |r| Ok((r.get(0)?,r.get(1)?)))?;
        if count >= 64
            || project_bytes + max_bytes as i64 > MAX_OWNER_BYTES
            || (class == "image"
                && (image_bytes + max_bytes as i64 > MAX_IMAGE_TOTAL_BYTES
                    || owner_images + max_bytes as i64 > MAX_IMAGE_OWNER_BYTES))
            || running >= 2
            || bytes + max_bytes as i64 > MAX_TOTAL_BYTES
            || owned + max_bytes as i64 > MAX_OWNER_BYTES
        {
            return Err(Error::ResourceExhausted);
        }
        let id = crate::broker::new_id()?;
        let metadata = serde_json::to_string(source).map_err(|_| Error::InvalidInput)?;
        tx.execute("INSERT INTO artifacts(id,pairing,project,reserved_bytes,state,expires,metadata,class) VALUES(?1,?2,?3,?4,'reserved',?5,?6,?7)",params![id,owner,project,max_bytes as i64,expires,metadata,class])?;
        tx.commit()?;
        let pin = std::sync::Arc::new(());
        self.artifact_pins
            .insert(id.clone(), std::sync::Arc::downgrade(&pin));
        Ok(Reservation {
            id,
            max_bytes,
            owner: owner.into(),
            project: project.into(),
            class,
            _pin: pin,
        })
    }
    pub fn staging_directory(&self, reservation: &Reservation) -> Result<fs::File> {
        let valid: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=?1 AND pairing=?2 AND project=?3 AND state='reserved' AND class='apk')", params![reservation.id,reservation.owner,reservation.project], |r| r.get(0))?;
        if !valid || reservation.class != "apk" {
            return Err(Error::TargetNotFound);
        }
        let path = self.root.join("artifacts");
        private(&path, true)?;
        Ok(fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?)
    }
    pub fn commit_import(
        &mut self,
        reservation: &Reservation,
        copy: crate::staging::StagedCopy,
        now: i64,
    ) -> Result<Artifact> {
        let row: Option<(String, i64, i64)> = self.connection.query_row("SELECT metadata,reserved_bytes,expires FROM artifacts WHERE id=?1 AND pairing=?2 AND project=?3 AND state='reserved' AND class='apk'", params![reservation.id,reservation.owner,reservation.project], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (source, reserved, expires) = row.ok_or(Error::TargetNotFound)?;
        let source: ArtifactSource =
            serde_json::from_str(&source).map_err(|_| Error::StorageUnavailable)?;
        if reservation.class != "apk"
            || copy.reservation_id() != reservation.id
            || copy.byte_length != reserved as u64
            || !source_valid(&source)
            || !matches!(source, ArtifactSource::Project(_))
            || now < 0
            || now >= expires
        {
            return Err(Error::InvalidInput);
        }
        copy.publish().map_err(|_| Error::StorageUnavailable)?;
        let artifact = Artifact {
            id: reservation.id.clone(),
            media_type: "application/vnd.android.package-archive".into(),
            byte_length: copy.byte_length as u32,
            sha256: copy.sha256.clone(),
            created_at_seconds: now.to_string(),
            expires_at_seconds: expires.to_string(),
            source,
            image: None,
        };
        drop(copy);
        // The temporary hard link has gone before publication in SQLite.
        private(&self.blob_path(&artifact.id)?, false)?;
        fs::File::open(self.root.join("artifacts"))?.sync_all()?;
        let metadata = serde_json::to_string(&artifact).map_err(|_| Error::InvalidInput)?;
        self.connection.execute(
            "UPDATE artifacts SET state='ready',metadata=?2 WHERE id=?1 AND state='reserved'",
            params![reservation.id, metadata],
        )?;
        Ok(artifact)
    }
    pub fn lease_artifact(
        &mut self,
        owner: &str,
        project: &str,
        id: &str,
        now: i64,
    ) -> Result<ArtifactFile> {
        let artifact = self.artifact_metadata(owner, project, id, now)?;
        let path = self.blob_path(id)?;
        private(path.parent().unwrap(), true)?;
        private(&path, false)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.mode() & 0o777 != 0o600
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.len() != u64::from(artifact.byte_length)
        {
            return Err(Error::StorageUnavailable);
        }
        let pin = self
            .artifact_pins
            .get(id)
            .and_then(std::sync::Weak::upgrade)
            .unwrap_or_else(|| std::sync::Arc::new(()));
        self.artifact_pins
            .insert(id.into(), std::sync::Arc::downgrade(&pin));
        Ok(ArtifactFile {
            file,
            artifact,
            _pin: pin,
        })
    }
    pub fn abandon_artifact(&mut self, reservation: &Reservation) -> Result<()> {
        let valid:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=?1 AND pairing=?2 AND project=?3 AND state='reserved')",params![reservation.id,reservation.owner,reservation.project],|r|r.get(0))?;
        if !valid {
            return Err(Error::TargetNotFound);
        }
        self.remove_artifact(&reservation.id)
    }
    pub fn commit_artifact(
        &mut self,
        reservation: &Reservation,
        bytes: &[u8],
        geometry: ImageGeometry,
        now: i64,
    ) -> Result<Artifact> {
        let metadata:Option<(String,i64,i64)>=self.connection.query_row("SELECT metadata,reserved_bytes,expires FROM artifacts WHERE id=?1 AND pairing=?2 AND project=?3 AND state='reserved'",params![reservation.id,reservation.owner,reservation.project],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (source, reserved, expires) = metadata.ok_or(Error::TargetNotFound)?;
        if reservation.class != "image"
            || bytes.len() > reserved as usize
            || bytes.len() > MAX_IMAGE_BYTES
            || bytes.len() < 24
            || !geometry_valid(&geometry)
            || now < 0
            || now >= expires
            || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
            || &bytes[12..16] != b"IHDR"
            || u32::from_be_bytes(bytes[16..20].try_into().unwrap()) != geometry.dimensions().0
            || u32::from_be_bytes(bytes[20..24].try_into().unwrap()) != geometry.dimensions().1
        {
            return Err(Error::InvalidInput);
        }
        let source: ArtifactSource =
            serde_json::from_str(&source).map_err(|_| Error::StorageUnavailable)?;
        if !source_valid(&source) || !classification_matches(&source, &geometry) {
            return Err(Error::StorageUnavailable);
        }
        let artifact = Artifact {
            id: reservation.id.clone(),
            media_type: "image/png".into(),
            byte_length: bytes.len() as u32,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            created_at_seconds: now.to_string(),
            expires_at_seconds: expires.to_string(),
            source,
            image: Some(geometry),
        };
        let path = self.blob_path(&reservation.id)?;
        private(path.parent().unwrap(), true)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::File::open(path.parent().unwrap())?.sync_all()?;
        let metadata = serde_json::to_string(&artifact).map_err(|_| Error::InvalidInput)?;
        self.connection.execute("UPDATE artifacts SET state='ready',reserved_bytes=?2,metadata=?3 WHERE id=?1 AND state='reserved'",params![reservation.id,bytes.len() as i64,metadata])?;
        Ok(artifact)
    }
    /// The broker must re-check the descriptor's source grant before reading bytes.
    pub fn artifact_metadata(
        &self,
        owner: &str,
        project: &str,
        id: &str,
        now: i64,
    ) -> Result<Artifact> {
        blob_id(id)?;
        let metadata:Option<String>=self.connection.query_row("SELECT metadata FROM artifacts WHERE id=?1 AND pairing=?2 AND project=?3 AND state='ready' AND expires>?4",params![id,owner,project,now],|r|r.get(0)).optional()?;
        let artifact: Artifact = serde_json::from_str(&metadata.ok_or(Error::TargetNotFound)?)
            .map_err(|_| Error::StorageUnavailable)?;
        let shape_valid = match (&artifact.source, &artifact.image) {
            (ArtifactSource::Project(_), None) => {
                artifact.media_type == "application/vnd.android.package-archive"
                    && (4..=crate::staging::MAX_IMPORT_BYTES)
                        .contains(&u64::from(artifact.byte_length))
            }
            (source, Some(image)) => {
                geometry_valid(image)
                    && classification_matches(source, image)
                    && artifact.byte_length as usize <= MAX_IMAGE_BYTES
                    && artifact.media_type == "image/png"
            }
            _ => false,
        };
        if artifact.id != id || !source_valid(&artifact.source) || !shape_valid {
            return Err(Error::StorageUnavailable);
        }
        Ok(artifact)
    }
    pub fn artifact_bytes(&self, artifact: &Artifact) -> Result<Vec<u8>> {
        if artifact.media_type != "image/png" || artifact.byte_length as usize > MAX_IMAGE_BYTES {
            return Err(Error::InvalidInput);
        }
        let path = self.blob_path(&artifact.id)?;
        private(path.parent().unwrap(), true)?;
        private(&path, false)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(Error::StorageUnavailable);
        }
        let mut bytes = Vec::new();
        file.take(MAX_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() != artifact.byte_length as usize
            || format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256
        {
            return Err(Error::StorageUnavailable);
        }
        Ok(bytes)
    }
}

static NATIVE_PRODUCERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Hold through native callback completion, including after the caller times out.
/// This bounds native captures across brokers, sessions and browser generations.
pub struct ProducerPermit;
impl ProducerPermit {
    pub fn acquire() -> std::result::Result<Self, lomi_control_protocol::ErrorCode> {
        use std::sync::atomic::Ordering;
        NATIVE_PRODUCERS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| lomi_control_protocol::ErrorCode::ResourceExhausted)?;
        Ok(Self)
    }
}
impl Drop for ProducerPermit {
    fn drop(&mut self) {
        NATIVE_PRODUCERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    fn source() -> ArtifactSource {
        ArtifactSource::Browser(BrowserArtifactSource {
            workspace_id: "workspace".into(),
            panel_id: "panel".into(),
            browser_generation: "generation".into(),
            profile_id: "profile".into(),
            navigation_id: "generation:1".into(),
            origin: "https://example.com".into(),
            required_scope: "browser.capture_composite".into(),
        })
    }
    fn image() -> (Vec<u8>, ImageGeometry) {
        (base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap(),
        ImageGeometry::Browser(BrowserImageGeometry {captured_at_millis:"100000".into(),css_width:1.,css_height:1.,device_scale_factor:1.,pixel_width:1,pixel_height:1,capture_scale_x:1.,capture_scale_y:1.,page_zoom:1.,scroll_x:0.,scroll_y:0.,crop:"viewport".into()}))
    }
    fn apk_source() -> ArtifactSource {
        ArtifactSource::Project(lomi_control_protocol::artifact::ProjectArtifactSource {
            workspace_id: "workspace".into(),
            relative_path: "build/app.apk".into(),
            required_scope: "files.read".into(),
        })
    }
    #[test]
    fn apk_copy_is_committed_once_and_active_lease_stays_in_the_expired_budget() {
        use crate::{project_files::ProjectDirectory, staging::StagedCopy};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::write(root.join("app.apk"), b"test APK bytes").unwrap();
        let project = ProjectDirectory::open(&root).unwrap();
        let mut store = Store::open(&root.join("control"), 100).unwrap();
        let reservation = store
            .reserve_artifact("owner", "project", &apk_source(), 14, 100)
            .unwrap();
        let copy = StagedCopy::copy(
            project.open_file("app.apk", 100).unwrap(),
            store.staging_directory(&reservation).unwrap(),
            &reservation.id,
            14,
            &format!("{:x}", Sha256::digest(b"test APK bytes")),
            || Ok(()),
        )
        .unwrap();
        assert!(matches!(
            store.artifact_metadata("owner", "project", &reservation.id, 101),
            Err(Error::TargetNotFound)
        ));
        let artifact = store.commit_import(&reservation, copy, 101).unwrap();
        drop(reservation);
        assert!(artifact.image.is_none());
        assert!(store.artifact_bytes(&artifact).is_err()); // APK bytes never enter MCP JSON/base64.
        assert!(store
            .lease_artifact("foreign", "project", &artifact.id, 102)
            .is_err());
        let mut lease = store
            .lease_artifact("owner", "project", &artifact.id, 102)
            .unwrap();
        fs::write(root.join("app.apk"), b"source changed").unwrap();
        lease.verify(|| Ok(())).unwrap();
        let path = store.blob_path(&artifact.id).unwrap();
        store.recover_artifacts(100 + RETENTION).unwrap();
        assert!(path.exists());
        let bytes: i64 = store
            .connection
            .query_row("SELECT SUM(reserved_bytes) FROM artifacts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(bytes, 14);
        assert!(store
            .artifact_metadata("owner", "project", &artifact.id, 100 + RETENTION)
            .is_err());
        drop(lease);
        store.recover_artifacts(100 + RETENTION).unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn apk_recovery_removes_only_durable_reservation_names_at_every_publish_boundary() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("control");
        let mut store = Store::open(&root, 100).unwrap();
        let r = store
            .reserve_artifact("owner", "project", &apk_source(), 1024, 100)
            .unwrap();
        let dir = root.join("artifacts");
        let stage = dir.join(format!("{}.stage", r.id));
        fs::write(&stage, b"owned").unwrap();
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&stage, dir.join(format!("{}.ready", r.id))).unwrap();
        fs::hard_link(&stage, dir.join(format!("{}.apk", r.id))).unwrap();
        fs::write(dir.join("unrecorded.stage"), b"preserve").unwrap();
        drop(r);
        drop(store);
        let _store = Store::open(&root, 102).unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        assert_eq!(fs::read(dir.join("unrecorded.stage")).unwrap(), b"preserve");
    }
    #[test]
    fn apk_limits_count_all_reserved_bytes_by_project_owner_and_globally() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("control"), 100).unwrap();
        let max = crate::staging::MAX_IMPORT_BYTES as usize;
        assert!(matches!(
            store.reserve_artifact("a", "p", &apk_source(), max + 1, 100),
            Err(Error::InvalidInput)
        ));
        let a = store
            .reserve_artifact("a", "p", &apk_source(), max, 100)
            .unwrap();
        let b = store
            .reserve_artifact("b", "p", &apk_source(), max, 100)
            .unwrap();
        // Simulate completed reservations to isolate quota from producer limits.
        store
            .connection
            .execute("UPDATE artifacts SET state='ready'", [])
            .unwrap();
        assert!(matches!(
            store.reserve_artifact("c", "p", &apk_source(), 4, 100),
            Err(Error::ResourceExhausted)
        ));
        let c = store
            .reserve_artifact("a", "q", &apk_source(), max, 100)
            .unwrap();
        store
            .connection
            .execute("UPDATE artifacts SET state='ready'", [])
            .unwrap();
        assert!(matches!(
            store.reserve_artifact("a", "r", &apk_source(), 4, 100),
            Err(Error::ResourceExhausted)
        ));
        let d = store
            .reserve_artifact("d", "s", &apk_source(), max, 100)
            .unwrap();
        store
            .connection
            .execute("UPDATE artifacts SET state='ready'", [])
            .unwrap();
        assert!(matches!(
            store.reserve_artifact("e", "t", &apk_source(), 4, 100),
            Err(Error::ResourceExhausted)
        ));
        drop((a, b, c, d));
    }
    #[test]
    fn reservations_bound_producers_and_immutable_bytes_preserve_classification() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("control"), 100).unwrap();
        let a = store
            .reserve_artifact("owner", "project", &source(), 1024, 100)
            .unwrap();
        let b = store
            .reserve_artifact("owner", "project", &source(), 1024, 100)
            .unwrap();
        assert!(matches!(
            store.reserve_artifact("other", "project", &source(), 1024, 100),
            Err(Error::ResourceExhausted)
        ));
        assert_eq!(
            store
                .artifact_metadata("owner", "project", &a.id, 100)
                .unwrap_err(),
            Error::TargetNotFound
        );
        let (bytes, geometry) = image();
        let artifact = store
            .commit_artifact(&a, &bytes, geometry.clone(), 101)
            .unwrap();
        assert_eq!(artifact.source, source());
        assert_eq!(store.artifact_bytes(&artifact).unwrap(), bytes);
        assert_eq!(
            store
                .artifact_metadata("other", "project", &a.id, 101)
                .unwrap_err(),
            Error::TargetNotFound
        );
        assert_eq!(
            store
                .artifact_metadata("owner", "other-project", &a.id, 101)
                .unwrap_err(),
            Error::TargetNotFound
        );
        assert_eq!(
            fs::metadata(store.blob_path(&a.id).unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(store.commit_artifact(&a, &bytes, geometry, 102).is_err());
        store.abandon_artifact(&b).unwrap();
        fs::write(store.blob_path(&a.id).unwrap(), b"changed").unwrap();
        assert_eq!(
            store.artifact_bytes(&artifact).unwrap_err(),
            Error::StorageUnavailable
        );
    }
    #[test]
    fn recovery_discards_incomplete_bytes_and_retains_ready_artifacts_only_until_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("control");
        let mut store = Store::open(&root, 100).unwrap();
        let incomplete = store
            .reserve_artifact("owner", "project", &source(), 1024, 100)
            .unwrap();
        let path = store.blob_path(&incomplete.id).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap()
            .write_all(b"partial")
            .unwrap();
        let ready = store
            .reserve_artifact("owner", "project", &source(), 1024, 100)
            .unwrap();
        let (bytes, geometry) = image();
        let artifact = store
            .commit_artifact(&ready, &bytes, geometry, 101)
            .unwrap();
        drop(store);
        let store = Store::open(&root, 102).unwrap();
        assert!(!path.exists());
        assert_eq!(
            store
                .artifact_bytes(
                    &store
                        .artifact_metadata("owner", "project", &artifact.id, 102)
                        .unwrap()
                )
                .unwrap(),
            bytes
        );
        drop(store);
        let store = Store::open(&root, 100 + RETENTION).unwrap();
        assert!(!root
            .join("artifacts")
            .join(format!("{}.png", artifact.id))
            .exists());
        assert_eq!(
            store
                .artifact_metadata("owner", "project", &artifact.id, 100 + RETENTION)
                .unwrap_err(),
            Error::TargetNotFound
        );
    }
    #[test]
    fn cleanup_refuses_a_redirected_directory_and_never_touches_the_foreign_file() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("control");
        let mut store = Store::open(&root, 100).unwrap();
        let a = store
            .reserve_artifact("owner", "project", &source(), 1024, 100)
            .unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let foreign = outside.join(format!("{}.png", a.id));
        fs::write(&foreign, b"preserve").unwrap();
        fs::remove_dir(root.join("artifacts")).unwrap();
        symlink(&outside, root.join("artifacts")).unwrap();
        assert_eq!(
            store.abandon_artifact(&a).unwrap_err(),
            Error::StorageUnavailable
        );
        drop(store);
        assert!(Store::open(&root, 101).is_err());
        assert_eq!(fs::read(foreign).unwrap(), b"preserve");
    }
}
