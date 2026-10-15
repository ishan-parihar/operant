//! Invariant 2 (BOUNDARY.md §5): every `result.json` a Holon body produces
//! validates against the kernel's own published JSON Schemas — vendored
//! here so the check runs offline and pins the kernel's contract shape.
//! The schemas are `holon contract schemas` output (the kernel regenerates
//! them from its own contract module); a drift test regenerates and
//! compares when HOLON_REPO is set.

use std::path::Path;

use serde_json::Value;

use crate::{Error, Result};

/// The stage a run's result.json belongs to, from the run's policy.json
/// (the kernel writes it there at claim).
pub fn validate_result(run_dir: &Path, stage: &str) -> Result<()> {
    let raw = std::fs::read_to_string(run_dir.join("result.json"))
        .map_err(|e| Error::Protocol(format!("result.json unreadable: {e}")))?;
    let doc: Value = serde_json::from_str(&raw)
        .map_err(|e| Error::PolicyParse(e))?;
    let schema_name = format!("{stage}-result.schema.json");
    // the schemas load from the vendored set at runtime (include_str
    // cannot be dynamic over the stage name)
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("vendor/schemas")
        .join(&schema_name);
    let schema_raw = std::fs::read_to_string(&schema_path)
        .map_err(|e| Error::Protocol(format!("schema {schema_name} unreadable: {e}")))?;
    let schema: Value = serde_json::from_str(&schema_raw).map_err(|e| Error::PolicyParse(e))?;
    // Structural validation of the invariant's half: the envelope must
    // satisfy at least one schema variant. The schemas are JSON Schema;
    // a full validator crate is out of scope for this seam — the kernel
    // re-validates at submit (its own code is the authority). Here we
    // refuse a result.json that matches NO variant's required set.
    let variants: Vec<Value> = schema
        .get("oneOf")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !variants.is_empty() {
        let matched = variants.iter().any(|v| {
            v.get("required")
                .and_then(Value::as_array)
                .map(|req| {
                    req.iter().all(|key| {
                        key.as_str().is_some_and(|k| doc.get(k).is_some())
                    })
                })
                .unwrap_or(false)
        });
        if !matched {
            return Err(Error::Protocol(format!(
                "result.json matches no {stage} schema variant — every variant requires its own keys (see vendor/schemas/{schema_name})"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant 2's seam: a result.json missing a required contract key
    /// is refused by name.
    #[test]
    fn a_result_missing_a_required_key_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        // plan schema requires summary + outcome (at minimum, per the
        // kernel's COMMON): write an envelope missing `outcome`.
        std::fs::write(
            dir.path().join("result.json"),
            r#"{"summary": "incomplete"}"#,
        )
        .expect("write");
        let err = validate_result(dir.path(), "plan").expect_err("must refuse");
        assert!(
            err.to_string().contains("matches no plan schema variant"),
            "named refusal expected, got {err}"
        );
    }

    #[test]
    fn a_complete_envelope_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        // v0 of the plan schema: kind + findings + steps
        std::fs::write(
            dir.path().join("result.json"),
            r#"{"kind": "deliver", "findings": "empty dir", "steps": ["step one"], "writes": []}"#,
        )
        .expect("write");
        validate_result(dir.path(), "plan").expect("a complete plan envelope passes");
    }
}
