use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub password_token: String,
    pub directory_services_id: String,
    pub name: String,
    pub store_front: String,
    pub pod: Option<String>,
    #[serde(default, skip)]
    pub password: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::Account;

    fn test_account() -> Account {
        Account {
            email: "user@example.com".to_string(),
            password_token: "token".to_string(),
            directory_services_id: "12345".to_string(),
            name: "Test User".to_string(),
            store_front: "143441-1,29".to_string(),
            pod: Some("31".to_string()),
            password: Some("super-secret".to_string()),
        }
    }

    #[test]
    fn skips_password_when_serializing() {
        let json = serde_json::to_string(&test_account()).unwrap();

        assert!(!json.contains(r#""password":"#));
        assert!(!json.contains("super-secret"));
    }

    #[test]
    fn ignores_password_when_deserializing() {
        let json = r#"{
            "email": "user@example.com",
            "password_token": "token",
            "directory_services_id": "12345",
            "name": "Test User",
            "store_front": "143441-1,29",
            "pod": "31",
            "password": "super-secret"
        }"#;

        let account: Account = serde_json::from_str(json).unwrap();

        assert_eq!(account.email, "user@example.com");
        assert!(account.password.is_none());
    }
}
