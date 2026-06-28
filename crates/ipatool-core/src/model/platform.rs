use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    #[serde(rename = "iphone")]
    IPhone,
    #[serde(rename = "ipad")]
    IPad,
    #[serde(rename = "appletv")]
    AppleTV,
}

impl Platform {
    pub fn search_entity(&self) -> &'static str {
        match self {
            Self::IPhone => "software",
            Self::IPad => "iPadSoftware",
            Self::AppleTV => "tvSoftware",
        }
    }

    pub fn lookup_entity(&self) -> &'static str {
        self.search_entity()
    }

    pub fn bundle_platform_name(&self) -> Option<&'static str> {
        match self {
            Self::AppleTV => Some("AppleTVOS"),
            _ => None,
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IPhone => write!(f, "iphone"),
            Self::IPad => write!(f, "ipad"),
            Self::AppleTV => write!(f, "appletv"),
        }
    }
}

impl std::str::FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "iphone" => Ok(Self::IPhone),
            "ipad" => Ok(Self::IPad),
            "appletv" | "apple_tv" => Ok(Self::AppleTV),
            other => Err(format!("unknown platform: {other}")),
        }
    }
}
