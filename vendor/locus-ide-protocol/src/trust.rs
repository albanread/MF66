use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum TaintOrigin {
    Host,
    Worker,
    Plugin { name: String },
    Mcp { server: String },
    External { label: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TrustLabel {
    pub trusted: bool,
    pub origin: TaintOrigin,
}

impl TrustLabel {
    pub fn host() -> Self {
        Self {
            trusted: true,
            origin: TaintOrigin::Host,
        }
    }

    pub fn worker() -> Self {
        Self {
            trusted: false,
            origin: TaintOrigin::Worker,
        }
    }

    pub fn plugin(name: impl Into<String>) -> Self {
        Self {
            trusted: false,
            origin: TaintOrigin::Plugin { name: name.into() },
        }
    }

    pub fn external(label: impl Into<String>) -> Self {
        Self {
            trusted: false,
            origin: TaintOrigin::External {
                label: label.into(),
            },
        }
    }
}
