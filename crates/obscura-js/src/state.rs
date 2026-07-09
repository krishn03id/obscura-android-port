//! state.rs — the shared state that ops read/write.
//!
//! Mirrors the original ObscuraState from ops.rs. Field names/types kept
//! identical so obscura-browser/obscura-cdp compile unchanged.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use obscura_dom::DomTree;
use obscura_net::{CookieJar, ObscuraHttpClient};
#[cfg(feature = "stealth")]
use obscura_net::StealthHttpClient;

use crate::ops::{InterceptedRequest, StoredNetworkResponseBody};

pub struct ObscuraState {
    pub dom: Option<DomTree>,
    pub url: String,
    pub encoding: String,
    pub title: String,
    pub blocked_urls: Vec<String>,
    pub cookie_jar: Option<Arc<CookieJar>>,
    pub http_client: Option<Arc<ObscuraHttpClient>>,
    #[cfg(feature = "stealth")]
    pub stealth_client: Option<Arc<StealthHttpClient>>,
    pub pending_navigation: Option<(String, String, String)>,
    pub intercept_tx: Option<tokio::sync::mpsc::UnboundedSender<InterceptedRequest>>,
    pub intercept_counter: u64,
    pub intercept_enabled: bool,
    pub pending_binding_calls: Vec<(String, String)>,
    pub network_response_bodies: HashMap<String, StoredNetworkResponseBody>,
    pub network_response_body_order: VecDeque<String>,
    pub network_response_body_counter: u64,
    pub fetched_urls: Vec<String>,
    /// Results from async ops (op_fetch_url) keyed by the async slot index.
    /// The event loop pump checks this and resolves the corresponding Promises.
    pub pending_async_results: HashMap<usize, Result<String, String>>,
}

impl ObscuraState {
    pub fn new() -> Self {
        ObscuraState {
            dom: None,
            url: "about:blank".to_string(),
            encoding: "UTF-8".to_string(),
            title: String::new(),
            blocked_urls: Vec::new(),
            cookie_jar: None,
            http_client: None,
            #[cfg(feature = "stealth")]
            stealth_client: None,
            pending_navigation: None,
            intercept_tx: None,
            intercept_counter: 0,
            intercept_enabled: false,
            pending_binding_calls: Vec::new(),
            network_response_bodies: HashMap::new(),
            network_response_body_order: VecDeque::new(),
            network_response_body_counter: 0,
            fetched_urls: Vec::new(),
            pending_async_results: HashMap::new(),
        }
    }
}

impl Default for ObscuraState {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedState = std::sync::Arc<std::sync::Mutex<ObscuraState>>;
