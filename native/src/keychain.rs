//! SSH key pairs stored in the local keychain.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    #[serde(default)]
    pub id: String,
}
