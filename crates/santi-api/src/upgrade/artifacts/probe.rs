use std::path::Path;
use std::process::Command;

use super::*;

impl Probe for Dpkg {
    fn inspect_deb(&self, path: &Path) -> Result<Identity, String> {
        Ok(Identity {
            package: deb_field(path, "Package")?,
            version: deb_field(path, "Version")?,
        })
    }

    fn installed(&self) -> Result<Identity, String> {
        let output = Command::new("dpkg-query")
            .args(["-W", "-f=${Package}\t${Version}", PACKAGE_NAME])
            .output()
            .map_err(|error| format!("query installed santi package: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "query installed santi package failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let value = String::from_utf8(output.stdout)
            .map_err(|error| format!("installed package identity is not UTF-8: {error}"))?;
        let (package, version) = value
            .trim()
            .split_once('\t')
            .ok_or_else(|| "installed package identity has an invalid shape".to_string())?;
        validate_identity(Identity {
            package: package.to_string(),
            version: version.to_string(),
        })
    }
}

fn deb_field(path: &Path, field: &str) -> Result<String, String> {
    let output = Command::new("dpkg-deb")
        .arg("--field")
        .arg(path)
        .arg(field)
        .output()
        .map_err(|error| format!("inspect {} field {field}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "inspect {} field {field} failed with {}: {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("{field} field is not UTF-8: {error}"))?;
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{} has an empty {field} field", path.display()))
    } else {
        Ok(value.to_string())
    }
}
