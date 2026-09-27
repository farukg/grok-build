//! Which provider route actually served a response, as reported by a routing gateway.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("route label must not be empty")]
pub struct EmptyRouteLabel;

macro_rules! route_label {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(raw: &str) -> Option<Self> {
                let trimmed = raw.trim();
                (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = EmptyRouteLabel;

            fn try_from(raw: String) -> Result<Self, Self::Error> {
                Self::parse(&raw).ok_or(EmptyRouteLabel)
            }
        }

        impl From<$name> for String {
            fn from(label: $name) -> Self {
                label.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    )+};
}

route_label! {
    /// Provider slug the gateway dispatched to (e.g. `openai`).
    RouteProvider,
    /// Provider-side model id that produced the response.
    RouteModel,
    /// Gateway candidate label that won routing.
    RouteCandidate,
    /// Non-secret account label of the provider identity that served the request.
    RouteIdentity,
    /// Gateway profile the requested model resolved to.
    RouteProfile,
}

/// Candidates tried and skipped before the serving one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FallbackCount(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServedRoute {
    pub provider: RouteProvider,
    pub model: RouteModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<RouteCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<RouteIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<RouteProfile>,
    #[serde(default)]
    pub fallback_count: FallbackCount,
}

impl ServedRoute {
    /// The short form plus the winning candidate and profile, for session details.
    pub fn detail(&self) -> ServedRouteDetail<'_> {
        ServedRouteDetail(self)
    }
}

pub struct ServedRouteDetail<'a>(&'a ServedRoute);

impl fmt::Display for ServedRouteDetail<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let route = self.0;
        write!(f, "{route}")?;
        if let Some(candidate) = &route.candidate {
            write!(f, " \u{b7} candidate {candidate}")?;
        }
        if let Some(profile) = &route.profile {
            write!(f, " \u{b7} profile {profile}")?;
        }
        Ok(())
    }
}

/// `provider/model`, then the identity and fallback count when present: `openai/gpt-5.6-sol · acct · fb=1`.
impl fmt::Display for ServedRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)?;
        if let Some(identity) = &self.identity {
            write!(f, " \u{b7} {identity}")?;
        }
        match self.fallback_count {
            FallbackCount(0) => Ok(()),
            FallbackCount(count) => write!(f, " \u{b7} fb={count}"),
        }
    }
}
