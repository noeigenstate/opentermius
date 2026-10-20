//! Port forwarding rules (local, remote and dynamic).
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Forward {
    #[serde(default)]
    pub id: String,
}
