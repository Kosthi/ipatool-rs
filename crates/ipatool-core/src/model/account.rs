use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub password_token: String,
    pub directory_services_id: String,
    pub name: String,
    pub store_front: String,
    pub pod: Option<String>,
}
