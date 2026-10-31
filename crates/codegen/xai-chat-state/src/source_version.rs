use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceVersion {
    #[serde(deserialize_with = "deserialize_incarnation")]
    pub incarnation: uuid::Uuid,
    pub sequence: u64,
}

fn deserialize_incarnation<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<uuid::Uuid, D::Error> {
    let incarnation = uuid::Uuid::deserialize(deserializer)?;
    if incarnation.is_nil() {
        return Err(serde::de::Error::custom("nil current-actions incarnation"));
    }
    Ok(incarnation)
}

#[cfg(test)]
#[path = "source_version_tests.rs"]
mod tests;
