use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserArtifactSource {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub profile_id: String,
    pub navigation_id: String,
    pub origin: String,
    pub required_scope: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserImageGeometry {
    pub captured_at_millis: String,
    pub css_width: f64,
    pub css_height: f64,
    pub device_scale_factor: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub capture_scale_x: f64,
    pub capture_scale_y: f64,
    pub page_zoom: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub crop: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidArtifactSource {
    pub workspace_id: String,
    pub panel_id: String,
    pub device_id: String,
    pub generation: String,
    pub required_scope: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectArtifactSource {
    #[serde(default)]
    pub kind: ArtifactImportKind,
    pub workspace_id: String,
    pub relative_path: String,
    pub required_scope: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ArtifactSource {
    Browser(BrowserArtifactSource),
    Android(AndroidArtifactSource),
    Project(ProjectArtifactSource),
}
impl ArtifactSource {
    pub fn workspace(&self) -> &str {
        match self {
            Self::Browser(s) => &s.workspace_id,
            Self::Android(s) => &s.workspace_id,
            Self::Project(s) => &s.workspace_id,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidImageGeometry {
    pub captured_at_millis: String,
    pub hardware_display: [u32; 2],
    pub rotation: u8,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub capture_scale_x: f64,
    pub capture_scale_y: f64,
    /// [a,b,c,d,tx,ty]: x=a*u+c*v+tx, y=b*u+d*v+ty; floor and clamp to hardware bounds.
    pub image_to_hardware: [f64; 6],
    pub coordinate_space: String,
    pub crop: String,
}
impl AndroidImageGeometry {
    pub fn transform(hardware: [u32; 2], width: u32, height: u32, rotation: u8) -> [f64; 6] {
        let [w, h] = hardware.map(f64::from);
        let (x, y) = (f64::from(width), f64::from(height));
        match rotation {
            0 => [w / x, 0., 0., h / y, 0., 0.],
            1 => [0., h / x, -w / y, 0., w, 0.],
            2 => [-w / x, 0., 0., -h / y, w, h],
            3 => [0., -h / x, w / y, 0., 0., h],
            _ => [f64::NAN; 6],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ImageGeometry {
    Browser(BrowserImageGeometry),
    Android(AndroidImageGeometry),
}
impl ImageGeometry {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Browser(g) => (g.pixel_width, g.pixel_height),
            Self::Android(g) => (g.pixel_width, g.pixel_height),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub id: String,
    pub media_type: String,
    pub byte_length: u32,
    pub sha256: String,
    pub created_at_seconds: String,
    pub expires_at_seconds: String,
    pub source: ArtifactSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageGeometry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserScreenshotInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    #[serde(default = "default_width")]
    pub max_width: u16,
    #[serde(default = "default_bytes")]
    pub max_bytes: u32,
}
fn default_width() -> u16 {
    1280
}
fn default_bytes() -> u32 {
    1024 * 1024
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactReadInput {
    pub workspace_id: String,
    pub artifact_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactImportInput {
    pub workspace_id: String,
    pub relative_path: String,
    pub kind: ArtifactImportKind,
    pub expected_byte_length: u64,
    pub expected_sha256: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactImportKind {
    #[default]
    AndroidApk,
    File,
}
impl ArtifactImportKind {
    pub fn scope(self) -> &'static str {
        match self {
            Self::AndroidApk => "artifact.import",
            Self::File => "artifact.import_file",
        }
    }
    pub fn max_bytes(self) -> u64 {
        match self {
            Self::AndroidApk => 512 * 1024 * 1024,
            Self::File => 4 * 1024 * 1024,
        }
    }
    pub fn min_bytes(self) -> u64 {
        if self == Self::AndroidApk {
            4
        } else {
            0
        }
    }
    pub fn media_type(self) -> &'static str {
        match self {
            Self::AndroidApk => "application/vnd.android.package-archive",
            Self::File => "application/octet-stream",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactExportInput {
    pub workspace_id: String,
    pub artifact_id: String,
    pub expected_sha256: String,
    pub relative_path: String,
    pub expected_parent_revision: String,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactExportCommand {
    pub workspace_id: String,
    pub not_after_millis: String,
    pub input: ArtifactExportInput,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactExported {
    pub workspace_id: String,
    pub relative_path: String,
    pub artifact: Box<Artifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserDownloadInput {
    pub workspace_id: String,
    pub panel_id: String,
    pub browser_generation: String,
    pub navigation_id: String,
    pub url: String,
    #[serde(default = "default_download_bytes")]
    pub max_bytes: u32,
    pub expected_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
fn default_download_bytes() -> u32 {
    4 * 1024 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_transform_matches_hardware_touch_for_each_rotation_and_scale() {
        for (rotation, expected) in [
            (0, [144., 384.]),
            (1, [504., 256.]),
            (2, [576., 896.]),
            (3, [216., 1024.]),
        ] {
            let [a, b, c, d, tx, ty] =
                AndroidImageGeometry::transform([720, 1280], 360, 640, rotation);
            let (u, v) = (360. * 0.2, 640. * 0.3);
            let actual = [a * u + c * v + tx, b * u + d * v + ty];
            assert!(actual
                .into_iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 1e-9));
        }
    }
    #[test]
    fn legacy_browser_artifact_shapes_remain_readable_without_new_tags() {
        let source = r#"{"workspaceId":"w","panelId":"p","browserGeneration":"g","profileId":"profile","navigationId":"nav","origin":"https://example.com","requiredScope":"browser.capture_composite"}"#;
        let parsed: ArtifactSource = serde_json::from_str(source).unwrap();
        assert!(matches!(parsed, ArtifactSource::Browser(_)));
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            serde_json::from_str::<serde_json::Value>(source).unwrap()
        );
    }
}
