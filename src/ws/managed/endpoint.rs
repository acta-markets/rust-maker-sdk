#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakerWsEndpoint {
    Quote,
    Data,
}

impl MakerWsEndpoint {
    const fn path(self) -> &'static str {
        match self {
            Self::Quote => "/maker",
            Self::Data => "/maker/data",
        }
    }
}

/// Normalize a base URL to a maker WebSocket endpoint.
#[must_use]
pub fn normalize_maker_ws_url_for_endpoint(url: &str, endpoint: MakerWsEndpoint) -> String {
    let url = url.trim().trim_end_matches('/');
    let url: std::borrow::Cow<'_, str> = if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}").into()
    } else if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}").into()
    } else {
        url.into()
    };
    let base = if let Some(base) = url.strip_suffix("/maker/data") {
        base
    } else if let Some(base) = url.strip_suffix("/maker") {
        base
    } else {
        url.as_ref()
    };
    format!("{}{}", base, endpoint.path())
}

#[must_use]
pub fn normalize_maker_ws_url(url: &str) -> String {
    normalize_maker_ws_url_for_endpoint(url, MakerWsEndpoint::Quote)
}

#[must_use]
pub fn normalize_maker_data_ws_url(url: &str) -> String {
    normalize_maker_ws_url_for_endpoint(url, MakerWsEndpoint::Data)
}
