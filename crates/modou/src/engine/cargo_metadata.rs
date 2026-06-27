use super::*;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

pub(crate) fn cargo_metadata(manifest_path: &Path) -> Result<Value, String> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .output()
        .map_err(|err| err.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    serde_json::from_slice(&output.stdout).map_err(|err| err.to_string())
}

pub(crate) fn find_package<'a>(metadata: &'a Value, package: &str) -> Option<&'a Value> {
    metadata["packages"]
        .as_array()?
        .iter()
        .find(|candidate| candidate["name"].as_str() == Some(package))
}

/// The names of the workspace's member crates. Because Modou runs
/// `cargo metadata --no-deps`, the `packages` array contains exactly the workspace
/// members (no transitive dependencies), so their names are the membership set used
/// by the workspace-scoped rule and by coverage. A `path` dependency that points
/// outside the workspace is therefore absent here, as intended.
pub(crate) fn workspace_member_names(metadata: &Value) -> Vec<String> {
    let mut names: Vec<String> = metadata["packages"]
        .as_array()
        .map(|packages| {
            packages
                .iter()
                .filter_map(|package| package["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

/// Whether a `cargo metadata` dependency belongs to the selected table. `kind` is
/// null for normal deps, `"dev"` / `"build"` otherwise.
fn kind_matches(dependency: &Value, kind: DependencyKind) -> bool {
    // An unrecognized `kind` string (none exist today — cargo emits only null/dev/build)
    // matches no `DependencyKind`, so such a dependency is observed by no boundary. This
    // is deliberate and bounded: `DependencyKind` does not grow (see its model doc), so a
    // new cargo table is a conscious amendment, not a silent gap to defend here.
    matches!(
        (kind, dependency["kind"].as_str()),
        (DependencyKind::Normal, None)
            | (DependencyKind::Dev, Some("dev"))
            | (DependencyKind::Build, Some("build"))
    )
}

/// Names of the target's dependencies in the selected table that resolve to a registry
/// or git source. Path/internal dependencies, and dependencies in other tables, are
/// excluded.
///
/// Names are package names, not local renames (`foo = { package = "bar" }` is
/// reported as `bar`), and platform-specific (`[target.'cfg(…)'.dependencies]`) and
/// `optional` deps are included — a declared dependency is governed as declared
/// (PROJECT.md).
pub(crate) fn external_dependencies(package: &Value, kind: DependencyKind) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(dependencies) = package["dependencies"].as_array() {
        for dependency in dependencies {
            if !kind_matches(dependency, kind) {
                continue;
            }
            // A path/internal dependency has a null `source`; any non-null source is
            // external. Match on presence, not on a fixed `registry+`/`git+` prefix
            // list, so a dependency from an alternative (e.g. `sparse+`) registry
            // cannot slip through unclassified and silently pass the boundary.
            let external = !dependency["source"].is_null();
            if external {
                // A dependency always carries a string `name` in cargo's metadata schema;
                // a present-but-non-string `name` (unexpected shape) is skipped rather
                // than failed. This relies on the schema guarantee — if it could be
                // violated, the loud path would be a scan error, not a silent skip.
                if let Some(name) = dependency["name"].as_str() {
                    found.push(name.to_string());
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Names of the target's dependencies in the selected table, regardless of source —
/// internal workspace path dependencies included. Used by the forbid and restrict-to
/// rules, which (unlike the external rule) must see internal crate-to-crate
/// dependencies. Same conventions as [`external_dependencies`]: package names (not
/// local renames), and platform-specific / `optional` deps are included (PROJECT.md).
pub(crate) fn dependencies(package: &Value, kind: DependencyKind) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(deps) = package["dependencies"].as_array() {
        for dependency in deps {
            if !kind_matches(dependency, kind) {
                continue;
            }
            if let Some(name) = dependency["name"].as_str() {
                found.push(name.to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}
