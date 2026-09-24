use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidPackageSelection {
    pub id: String,
    pub revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidSetupPlanInput {
    pub workspace_id: String,
    pub action: AndroidSetupQuery,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AndroidSetupQuery {
    Inventory,
    Catalog {
        #[serde(default)]
        refresh: bool,
        offset: u32,
        limit: u16,
        expected_catalog_revision: Option<String>,
    },
    Prepare {
        catalog_revision: String,
        packages: Vec<AndroidPackageSelection>,
        prepare_tools: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidDownload {
    pub id: String,
    pub revision: String,
    pub name: String,
    pub url: String,
    pub bytes: u64,
    pub checksum: String,
    pub license_id: Option<String>,
    pub dependencies: Vec<AndroidPackageSelection>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidLicenseSummary {
    pub id: String,
    pub digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidPreparedPlan {
    pub plan_id: String,
    pub revision: String,
    pub downloads: Vec<AndroidDownload>,
    pub licenses: Vec<AndroidLicenseSummary>,
    pub download_bytes: u64,
    pub target: String,
    pub expires_in_seconds: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidHardware {
    pub ram_mib: u32,
    pub cpu_count: u32,
    pub data_gib: u32,
    pub gpu: AndroidGpu,
    pub quick_boot: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AndroidGpu {
    Auto,
    Host,
    Software,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidManagedDevice {
    pub device_id: String,
    pub name: String,
    pub image: String,
    pub image_revision: String,
    pub profile: String,
    pub hardware: AndroidHardware,
    pub generation: Option<String>,
    pub process_alive: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidProfile {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub min_api: u32,
    pub min_minor_api: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AndroidSetupView {
    Inventory {
        devices_revision: Option<String>,
        manifest_revision: Option<String>,
        host_qualified: bool,
        devices: Vec<AndroidManagedDevice>,
        installed: Vec<AndroidPackageSelection>,
        profiles: Vec<AndroidProfile>,
        recovery_required: bool,
        recovery: Vec<AndroidMetadataRecovery>,
        rollbacks: Vec<AndroidPackageSelection>,
    },
    Catalog {
        catalog_revision: String,
        packages: Vec<AndroidDownload>,
        next_offset: Option<u32>,
    },
    Prepared(AndroidPreparedPlan),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidSetupApplyInput {
    pub workspace_id: String,
    pub plan_id: String,
    pub plan_revision: String,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidDeviceManageInput {
    pub workspace_id: String,
    pub action: AndroidDeviceAction,
    pub retry_epoch: String,
    pub request_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AndroidDeviceAction {
    Create {
        expected_devices_revision: String,
        name: String,
        image: String,
        profile: String,
        hardware: AndroidHardware,
    },
    Modify {
        expected_devices_revision: String,
        device_id: String,
        generation: Option<String>,
        name: String,
        profile: String,
        hardware: AndroidHardware,
    },
    Wipe {
        expected_devices_revision: String,
        device_id: String,
        generation: Option<String>,
        confirmation: String,
    },
    Delete {
        expected_devices_revision: String,
        device_id: String,
        generation: Option<String>,
        confirmation: String,
    },
    Recover,
    Cleanup,
    RestoreMetadata {
        file: AndroidMetadataFile,
        digest: String,
        reset: bool,
    },
    RemovePackage {
        package_id: String,
        expected_manifest_revision: String,
    },
    RollbackPackage {
        package_id: String,
        expected_manifest_revision: String,
    },
}
impl AndroidDeviceAction {
    pub fn device_id(&self) -> Option<&str> {
        match self {
            Self::Modify { device_id, .. }
            | Self::Wipe { device_id, .. }
            | Self::Delete { device_id, .. } => Some(device_id),
            _ => None,
        }
    }
    pub fn destructive_confirmation(&self) -> Option<&str> {
        match self {
            Self::Wipe { confirmation, .. } | Self::Delete { confirmation, .. } => {
                Some(confirmation)
            }
            Self::RestoreMetadata {
                file: AndroidMetadataFile::Devices,
                reset: true,
                ..
            } => Some("RESET DEVICES"),
            Self::RestoreMetadata {
                file: AndroidMetadataFile::Preferences,
                reset: true,
                ..
            } => Some("RESET PREFERENCES"),
            _ => None,
        }
    }
    pub fn scope(&self) -> &'static str {
        match self {
            Self::Recover
            | Self::Cleanup
            | Self::RestoreMetadata { .. }
            | Self::RemovePackage { .. }
            | Self::RollbackPackage { .. } => "android.setup",
            _ => "android.manage",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidManagementResult {
    pub workspace_id: String,
    pub native_operation_id: String,
    pub device_id: Option<String>,
    pub devices_revision: Option<String>,
    pub manifest_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AndroidMetadataFile {
    Devices,
    Preferences,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidMetadataRecovery {
    pub file: AndroidMetadataFile,
    pub digest: String,
    pub backup_revision: Option<String>,
}
