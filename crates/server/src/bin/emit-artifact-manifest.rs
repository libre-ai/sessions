//! Emits a depot-conformant ArtifactManifest for a presto-server release binary.
//!
//! Usage:
//!   cargo run --bin emit-artifact-manifest -- PATH_TO_BINARY [--json-out PATH]
//!
//! The manifest conforms to `gear-depot` schema v0.1:
//! - artifact_id, manifest_id, manifest_ref: derived from binary hash
//! - checksums: real SHA256 of the binary
//! - provenance_id: identifies the build source
//! - artifact_type: ReleaseAsset
//! - state: Active
//! - created_at: current timestamp in RFC3339

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tracing::{error, info};

/// Mimics the gear-depot ArtifactType for serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactType {
    SpecPackage,
    HandoffPayload,
    CuratedExport,
    LearningExport,
    ReleaseAsset,
    InspectionReport,
}

/// Mimics the gear-depot ArtifactState for serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactState {
    Active,
    Revoked,
    Superseded,
    Deleted,
}

/// Mimics the gear-depot PackageType for serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PackageType {
    JsonBundle,
    Tar,
    Zip,
    Binary,
    Container,
    RegistryPackage,
}

/// Mimics the gear-depot ChecksumAlgorithm for serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChecksumAlgorithm {
    Sha256,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checksum {
    algorithm: ChecksumAlgorithm,
    value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RetentionMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    policy_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revoked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delete_after: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DistributionMetadata {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    channels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    install_floor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactRef {
    artifact_id: String,
    artifact_type: ArtifactType,
    producer: String,
    version: String,
    hash: String,
    manifest_ref: String,
    state: ArtifactState,
    created_at: String,
}

/// SafeMetadata: simple key-value string map.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SafeMetadata {
    values: BTreeMap<String, String>,
}

impl SafeMetadata {
    fn new() -> Self {
        SafeMetadata {
            values: BTreeMap::new(),
        }
    }

    fn insert(&mut self, key: String, value: String) {
        self.values.insert(key, value);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactManifest {
    manifest_id: String,
    artifact: ArtifactRef,
    package_type: PackageType,
    checksums: Vec<Checksum>,
    provenance_id: String,
    retention: RetentionMetadata,
    distribution: DistributionMetadata,
    metadata: SafeMetadata,
}

fn compute_sha256_file(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    Ok(format!("sha256:{}", hex::encode(result)))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        error!("usage error: invalid argument count");
        std::process::exit(1);
    }

    let binary_path = Path::new(&args[1]);
    if !binary_path.exists() {
        error!(path = %binary_path.display(), "binary not found");
        std::process::exit(1);
    }

    let output_path = if args.len() > 2 && args[2] == "--json-out" {
        Some(Path::new(&args[3]).to_path_buf())
    } else {
        None
    };

    // Compute SHA256 hash.
    let hash = compute_sha256_file(binary_path)?;

    // Extract version from presto-server binary (use env var for now, default to 0.0.0).
    let version = std::env::var("ARTIFACT_VERSION").unwrap_or_else(|_| "0.0.0".to_string());

    // Get current timestamp in RFC3339.
    let now = time::OffsetDateTime::now_utc();
    let created_at = now.format(&time::format_description::well_known::Rfc3339)?;

    // Derive IDs from the hash (stable, deterministic).
    let artifact_id = format!("artifact:{}", hash);
    let manifest_id = format!("manifest:{}", hash);
    let provenance_id = format!(
        "provenance:rumble-lm:presto-server:{}",
        created_at.split('T').next().unwrap_or("unknown-date")
    );

    let mut metadata = SafeMetadata::new();
    metadata.insert("artifact_type".to_string(), "release_binary".to_string());
    metadata.insert("product".to_string(), "rumble-lm".to_string());
    metadata.insert(
        "binary_name".to_string(),
        binary_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("presto-server")
            .to_string(),
    );

    let manifest = ArtifactManifest {
        manifest_id: manifest_id.clone(),
        artifact: ArtifactRef {
            artifact_id,
            artifact_type: ArtifactType::ReleaseAsset,
            producer: "rumble-lm".to_string(),
            version,
            hash,
            manifest_ref: manifest_id.clone(),
            state: ArtifactState::Active,
            created_at,
        },
        package_type: PackageType::Binary,
        checksums: vec![Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: compute_sha256_file(binary_path)?,
        }],
        provenance_id,
        retention: RetentionMetadata::default(),
        distribution: DistributionMetadata::default(),
        metadata,
    };

    // Validate required fields.
    validate_manifest(&manifest)?;

    // Serialize to JSON.
    let json = serde_json::to_string_pretty(&manifest)?;

    if let Some(out_path) = output_path {
        fs::write(&out_path, &json)?;
        info!(path = %out_path.display(), "manifest written");
    } else {
        println!("{}", json); // allow-println: CLI output (--json-out not set)
    }

    info!("manifest is valid and conforms to gear-depot schema");
    Ok(())
}

/// Validates that the manifest conforms to gear-depot constraints.
fn validate_manifest(manifest: &ArtifactManifest) -> Result<(), Box<dyn std::error::Error>> {
    // manifest_id must not be empty.
    if manifest.manifest_id.is_empty() {
        return Err("manifest_id must not be empty".into());
    }

    // provenance_id must not be empty.
    if manifest.provenance_id.is_empty() {
        return Err("provenance_id must not be empty".into());
    }

    // artifact.artifact_id must not be empty.
    if manifest.artifact.artifact_id.is_empty() {
        return Err("artifact.artifact_id must not be empty".into());
    }

    // artifact.producer must not be empty.
    if manifest.artifact.producer.is_empty() {
        return Err("artifact.producer must not be empty".into());
    }

    // artifact.version must not be empty.
    if manifest.artifact.version.is_empty() {
        return Err("artifact.version must not be empty".into());
    }

    // artifact.manifest_ref must match manifest_id.
    if manifest.artifact.manifest_ref != manifest.manifest_id {
        return Err(format!(
            "artifact.manifest_ref '{}' does not match manifest_id '{}'",
            manifest.artifact.manifest_ref, manifest.manifest_id
        )
        .into());
    }

    // Must have at least one SHA256 checksum.
    let has_sha256 = manifest
        .checksums
        .iter()
        .any(|c| c.algorithm == ChecksumAlgorithm::Sha256);
    if !has_sha256 {
        return Err("manifest requires at least one sha256 checksum".into());
    }

    // Validate checksum format (must look like "sha256:..." — 7 + 64 hex chars).
    for checksum in &manifest.checksums {
        if checksum.algorithm == ChecksumAlgorithm::Sha256
            && (!checksum.value.starts_with("sha256:") || checksum.value.len() != 71)
        {
            return Err(format!(
                "checksum value '{}' is not a valid sha256 hash",
                checksum.value
            )
            .into());
        }
    }

    // artifact.hash must be non-empty.
    if manifest.artifact.hash.is_empty() {
        return Err("artifact.hash must not be empty".into());
    }

    // created_at must parse as RFC3339.
    time::PrimitiveDateTime::parse(
        &manifest.artifact.created_at,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|e| format!("artifact.created_at is not RFC3339: {}", e))?;

    Ok(())
}

// Implement FromStr for hex::encode-like functionality
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}
