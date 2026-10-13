use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "catalog_icons";

#[derive(Serialize, Deserialize)]
pub struct CatalogIcon {
    #[serde(rename = "_id")]
    pub id: String,
    pub svg: String,
}
