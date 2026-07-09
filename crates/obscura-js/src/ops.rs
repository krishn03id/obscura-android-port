//! ops.rs — shared state, types, and all JS↔Rust bridge ops.
//!
//! In the deno_core build these were #[op2] functions registered via an Extension.
//! In the rquickjs port they are plain Rust functions/closures installed onto
//! `globalThis.Deno.core.ops.*` by `install_ops()`.

use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use obscura_dom::{DomTree, NodeData, NodeId};
use obscura_net::{RequestInfo, ResourceType, Response};
#[cfg(feature = "stealth")]
use obscura_net::StealthHttpClient;
use rquickjs::{Ctx, Object, function::Func};
use tokio::sync::Mutex;

use crate::state::SharedState;

// ---------------------------------------------------------------------------
// Types: network interception & response storage
// ---------------------------------------------------------------------------

pub type InterceptCallback = Arc<Mutex<Option<Box<dyn Fn(String, String, String) -> Option<(u16, String, String)> + Send + Sync>>>>;

#[derive(Debug)]
pub enum InterceptResolution {
    Continue {
        url: Option<String>,
        method: Option<String>,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    },
    Fulfill {
        status: u16,
        headers: HashMap<String, String>,
        body: String,
    },
    Fail { reason: String },
}

pub struct InterceptedRequest {
    pub request_id: String,
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub resource_type: String,
    pub resolver: tokio::sync::oneshot::Sender<InterceptResolution>,
}

#[derive(Debug, Clone)]
pub struct StoredNetworkResponseBody {
    pub body: String,
    pub base64_encoded: bool,
}



// ---------------------------------------------------------------------------
// DOM ops: op_dom dispatches ~40 DOM commands (querySelector, attrs, etc.)
// ---------------------------------------------------------------------------

pub fn op_dom_impl(state: &SharedState, cmd: &str, arg1: &str, arg2: &str) -> String {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        op_dom_inner(state, cmd, arg1, arg2)
    }))
    .unwrap_or_else(|_| {
        tracing::error!("op_dom panicked; returning null");
        "null".to_string()
    })
}

fn op_dom_inner(state: &SharedState, cmd: &str, arg1: &str, arg2: &str) -> String {
    let gs = state.lock().unwrap();
    let dom = match &gs.dom {
        Some(d) => d,
        None => return "null".to_string(),
    };

    match cmd {
        "document_node_id" => dom.document().index().to_string(),
        "document_title" => serde_json::to_string(&gs.title).unwrap_or("\"\"".into()),
        "document_url" => serde_json::to_string(&gs.url).unwrap_or("\"\"".into()),
        "document_encoding" => serde_json::to_string(&gs.encoding).unwrap_or("\"UTF-8\"".into()),
        "document_element" => {
            for cid in dom.children(dom.document()) {
                if let Some(n) = dom.get_node(cid) {
                    if n.as_element().map(|name| name.local.as_ref() == "html").unwrap_or(false) {
                        return cid.index().to_string();
                    }
                }
            }
            "-1".into()
        }
        "document_doctype" => {
            for cid in dom.children(dom.document()) {
                if let Some(n) = dom.get_node(cid) {
                    if let NodeData::Doctype { name, public_id, system_id } = &n.data {
                        return serde_json::json!({
                            "name": name,
                            "publicId": public_id,
                            "systemId": system_id,
                            "nodeId": cid.index(),
                        }).to_string();
                    }
                }
            }
            "null".into()
        }
        "get_element_by_id" => {
            let doc = dom.document();
            let nid = dom.get_element_by_id(arg1);
            let live = nid.filter(|&n| dom.ancestors(n).contains(&doc));
            match live {
                Some(n) => n.index().to_string(),
                None => {
                    let sel = format!("[id=\"{}\"]", arg1.replace('\\', "\\\\").replace('"', "\\\""));
                    dom.query_selector(&sel).ok().flatten()
                        .map(|id| id.index().to_string()).unwrap_or("-1".into())
                }
            }
        }
        "query_selector" => {
            dom.query_selector(arg1).ok().flatten().map(|id| id.index().to_string()).unwrap_or("-1".into())
        }
        "query_selector_all" => {
            let ids: Vec<i32> = dom.query_selector_all(arg1).ok()
                .map(|ids| ids.iter().map(|id| id.index() as i32).collect()).unwrap_or_default();
            serde_json::to_string(&ids).unwrap_or("[]".into())
        }
        "query_selector_scoped" => {
            let root_nid = arg1.parse::<u32>().unwrap_or(0);
            dom.query_selector_from(NodeId::new(root_nid), arg2).ok().flatten()
                .map(|id| id.index().to_string()).unwrap_or("-1".into())
        }
        "query_selector_all_scoped" => {
            let root_nid = arg1.parse::<u32>().unwrap_or(0);
            let ids: Vec<i32> = dom.query_selector_all_from(NodeId::new(root_nid), arg2).ok()
                .map(|ids| ids.iter().map(|id| id.index() as i32).collect()).unwrap_or_default();
            serde_json::to_string(&ids).unwrap_or("[]".into())
        }
        "node_type" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            dom.with_node(NodeId::new(nid), |n| match &n.data {
                NodeData::Document => "9", NodeData::Element { .. } => "1", NodeData::Text { .. } => "3",
                NodeData::Comment { .. } => "8", NodeData::Doctype { .. } => "10", NodeData::ProcessingInstruction { .. } => "7",
            }).unwrap_or("0").into()
        }
        "node_name" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let name: String = dom.with_node(NodeId::new(nid), |n| match &n.data {
                NodeData::Document => "#document".to_string(), NodeData::Element { name, .. } => name.local.as_ref().to_ascii_uppercase(),
                NodeData::Text { .. } => "#text".to_string(), NodeData::Comment { .. } => "#comment".to_string(),
                NodeData::Doctype { name, .. } => name.clone(), NodeData::ProcessingInstruction { target, .. } => target.clone(),
            }).unwrap_or_default();
            serde_json::to_string(&name).unwrap_or("\"\"".into())
        }
        "text_content" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            serde_json::to_string(&dom.text_content(NodeId::new(nid))).unwrap_or("\"\"".into())
        }
        "parent_node" | "first_child" | "last_child" | "next_sibling" | "prev_sibling" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            dom.with_node(NodeId::new(nid), |n| match cmd {
                "parent_node" => n.parent, "first_child" => n.first_child,
                "last_child" => n.last_child, "next_sibling" => n.next_sibling,
                "prev_sibling" => n.prev_sibling, _ => None,
            }).flatten().map(|id| id.index().to_string()).unwrap_or("-1".into())
        }
        "child_nodes" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let ids: Vec<i32> = dom.children(NodeId::new(nid)).iter().map(|id| id.index() as i32).collect();
            serde_json::to_string(&ids).unwrap_or("[]".into())
        }
        "tag_name" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let name = dom.with_node(NodeId::new(nid), |n| n.as_element().map(|name| name.local.as_ref().to_ascii_uppercase())).flatten().unwrap_or_default();
            serde_json::to_string(&name).unwrap_or("\"\"".into())
        }
        "get_attribute" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let val = dom.with_node(NodeId::new(nid), |n| n.get_attribute(arg2).map(|s| s.to_string())).flatten();
            serde_json::to_string(&val).unwrap_or("null".into())
        }
        "attribute_names" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let names: Vec<String> = dom
                .with_node(NodeId::new(nid), |n| {
                    n.attrs()
                        .map(|a| a.iter().map(|x| x.name.local.as_ref().to_string()).collect())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            serde_json::to_string(&names).unwrap_or("[]".into())
        }
        "set_attribute" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let node_id = NodeId::new(nid);
            if let Some((name, value)) = arg2.split_once('\0') {
                if name == "id" {
                    let old_id = dom.with_node(node_id, |n| n.get_attribute("id").map(|s| s.to_string())).flatten();
                    dom.with_node_mut(node_id, |n| n.set_attribute(name, value.to_string()));
                    dom.update_id_index(node_id, old_id.as_deref(), Some(value));
                } else {
                    dom.with_node_mut(node_id, |n| n.set_attribute(name, value.to_string()));
                }
            }
            "true".into()
        }
        "inner_html" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            serde_json::to_string(&dom.inner_html(NodeId::new(nid))).unwrap_or("\"\"".into())
        }
        "outer_html" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            serde_json::to_string(&dom.outer_html(NodeId::new(nid))).unwrap_or("\"\"".into())
        }
        "append_child" => {
            let parent = match arg1.parse::<u32>() { Ok(n) => n, Err(_) => return "false".into() };
            let child = match arg2.parse::<u32>() { Ok(n) => n, Err(_) => return "false".into() };
            dom.append_child(NodeId::new(parent), NodeId::new(child));
            "true".into()
        }
        "remove_child" => {
            let child = match arg1.parse::<u32>() { Ok(n) => n, Err(_) => return "false".into() };
            dom.remove_child(NodeId::new(child));
            "true".into()
        }
        "insert_before" => {
            let new_node = match arg1.parse::<u32>() { Ok(n) => n, Err(_) => return "false".into() };
            let ref_node = match arg2.parse::<u32>() { Ok(n) => n, Err(_) => return "false".into() };
            dom.insert_before(NodeId::new(ref_node), NodeId::new(new_node));
            "true".into()
        }
        "remove_attribute" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            dom.with_node_mut(NodeId::new(nid), |n| {
                if let NodeData::Element { attrs, .. } = &mut n.data {
                    attrs.retain(|a| a.name.local.as_ref() != arg2);
                }
            });
            "true".into()
        }
        "set_inner_html" => {
            let nid = match arg1.parse::<u32>() {
                Ok(n) if n > 0 => n,
                _ => return "false".into(),
            };
            let target = NodeId::new(nid);
            let children = dom.children(target);
            for child in children {
                dom.detach(child);
            }
            if !arg2.is_empty() {
                let fragment = obscura_dom::parse_fragment(arg2);
                let import_root = fragment.find_body_or_root();
                dom.import_children_from(target, &fragment, import_root);
            }
            "true".into()
        }
        "set_text_content" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            dom.with_node_mut(NodeId::new(nid), |n| {
                match &mut n.data {
                    NodeData::Text { contents } => { *contents = arg2.to_string(); }
                    NodeData::Comment { contents } => { *contents = arg2.to_string(); }
                    NodeData::ProcessingInstruction { data, .. } => { *data = arg2.to_string(); }
                    _ => {}
                }
            });
            "true".into()
        }
        "create_document_fragment" => {
            dom.new_node(NodeData::Document).index().to_string()
        }
        "create_element" => {
            dom.new_node(NodeData::Element {
                name: html5ever::QualName::new(None, html5ever::ns!(html), html5ever::LocalName::from(arg1)),
                attrs: vec![], template_contents: None, mathml_annotation_xml_integration_point: false,
            }).index().to_string()
        }
        "create_text_node" => {
            dom.new_node(NodeData::Text { contents: arg1.to_string() }).index().to_string()
        }
        "create_comment_node" => {
            dom.new_node(NodeData::Comment { contents: arg1.to_string() }).index().to_string()
        }
        "create_processing_instruction" => {
            dom.new_node(NodeData::ProcessingInstruction {
                target: arg1.to_string(),
                data: arg2.to_string(),
            }).index().to_string()
        }
        "create_doctype" => {
            dom.new_node(NodeData::Doctype {
                name: arg1.to_string(),
                public_id: arg2.to_string(),
                system_id: String::new(),
            }).index().to_string()
        }
        "pi_target" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let val = dom.with_node(NodeId::new(nid), |n| match &n.data {
                NodeData::ProcessingInstruction { target, .. } => Some(target.clone()),
                _ => None,
            }).flatten().unwrap_or_default();
            serde_json::to_string(&val).unwrap_or("\"\"".into())
        }
        "doctype_name" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let val = dom.with_node(NodeId::new(nid), |n| match &n.data {
                NodeData::Doctype { name, .. } => Some(name.clone()),
                _ => None,
            }).flatten().unwrap_or_default();
            serde_json::to_string(&val).unwrap_or("\"\"".into())
        }
        "doctype_public_id" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let val = dom.with_node(NodeId::new(nid), |n| match &n.data {
                NodeData::Doctype { public_id, .. } => Some(public_id.clone()),
                _ => None,
            }).flatten().unwrap_or_default();
            serde_json::to_string(&val).unwrap_or("\"\"".into())
        }
        "element_children" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let ids: Vec<i32> = dom.children(NodeId::new(nid)).iter()
                .filter(|&&id| dom.get_node(id).map(|n| n.is_element()).unwrap_or(false))
                .map(|id| id.index() as i32).collect();
            serde_json::to_string(&ids).unwrap_or("[]".into())
        }
        "has_child_nodes" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            dom.with_node(NodeId::new(nid), |n| n.first_child.is_some()).unwrap_or(false).to_string()
        }
        "contains" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            let other = arg2.parse::<u32>().unwrap_or(0);
            dom.descendants(NodeId::new(nid)).contains(&NodeId::new(other)).to_string()
        }
        "node_index" => {
            let nid = arg1.parse::<u32>().unwrap_or(0);
            node_child_index(dom, NodeId::new(nid)).to_string()
        }
        "compare_order" => {
            let a = NodeId::new(arg1.parse::<u32>().unwrap_or(0));
            let b = NodeId::new(arg2.parse::<u32>().unwrap_or(0));
            compare_node_order(dom, a, b).to_string()
        }
        "node_root" => {
            let mut cur = NodeId::new(arg1.parse::<u32>().unwrap_or(0));
            while let Some(p) = dom.with_node(cur, |x| x.parent).flatten() {
                cur = p;
            }
            cur.index().to_string()
        }
        _ => "null".into(),
    }
}

fn node_child_index(dom: &DomTree, n: NodeId) -> usize {
    let mut i = 0usize;
    let mut cur = dom.with_node(n, |x| x.prev_sibling).flatten();
    while let Some(p) = cur {
        i += 1;
        cur = dom.with_node(p, |x| x.prev_sibling).flatten();
    }
    i
}

fn node_ancestors_root_first(dom: &DomTree, n: NodeId) -> Vec<NodeId> {
    let mut v = vec![n];
    let mut cur = n;
    while let Some(p) = dom.with_node(cur, |x| x.parent).flatten() {
        v.push(p);
        cur = p;
    }
    v.reverse();
    v
}

fn compare_node_order(dom: &DomTree, a: NodeId, b: NodeId) -> i32 {
    if a == b { return 0; }
    let aa = node_ancestors_root_first(dom, a);
    let bb = node_ancestors_root_first(dom, b);
    if aa[0] != bb[0] {
        return if a.index() < b.index() { -1 } else { 1 };
    }
    let mut i = 0usize;
    while i < aa.len() && i < bb.len() && aa[i] == bb[i] {
        i += 1;
    }
    if i >= aa.len() { return -1; }
    if i >= bb.len() { return 1; }
    if node_child_index(dom, aa[i]) < node_child_index(dom, bb[i]) { -1 } else { 1 }
}

// ---------------------------------------------------------------------------
// State ops: console logging, cookies, navigation, binding callbacks
// ---------------------------------------------------------------------------

pub fn op_console_msg_impl(level: &str, msg: &str) {
    match level {
        "warn" => tracing::warn!(target: "obscura::console", "{}", msg),
        "error" => tracing::error!(target: "obscura::console", "{}", msg),
        _ => tracing::info!(target: "obscura::console", "{}", msg),
    }
}

pub fn op_get_cookies_impl(state: &SharedState) -> String {
    let gs = state.lock().unwrap();
    let jar = match &gs.cookie_jar {
        Some(j) => j,
        None => return String::new(),
    };
    let url = match url::Url::parse(&gs.url) {
        Ok(u) => u,
        Err(_) => return String::new(),
    };
    jar.get_js_visible_cookies(&url)
}

pub fn op_set_cookie_impl(state: &SharedState, cookie_str: &str) {
    let gs = state.lock().unwrap();
    let jar = match &gs.cookie_jar {
        Some(j) => j,
        None => return,
    };
    let url = match url::Url::parse(&gs.url) {
        Ok(u) => u,
        Err(_) => return,
    };
    jar.set_cookie_from_js(cookie_str, &url);
}

pub fn op_navigate_impl(state: &SharedState, url: &str, method: &str, body: &str) {
    let mut gs = state.lock().unwrap();
    gs.url = url.to_string();
    gs.pending_navigation = Some((url.to_string(), method.to_string(), body.to_string()));
}

pub fn op_binding_called_impl(state: &SharedState, name: &str, payload: &str) {
    let mut gs = state.lock().unwrap();
    gs.pending_binding_calls.push((name.to_string(), payload.to_string()));
}

// ---------------------------------------------------------------------------
// URL ops: parse, set, resolve
// ---------------------------------------------------------------------------

pub fn op_url_parse_impl(href: &str, base: &str) -> String {
    std::panic::catch_unwind(|| {
        let parsed = if base.is_empty() {
            url::Url::parse(href)
        } else {
            url::Url::parse(base).and_then(|b| b.join(href))
        };
        match parsed {
            Ok(u) => url_components(&u).to_string(),
            Err(_) => "{\"ok\":false}".to_string(),
        }
    })
    .unwrap_or_else(|_| "{\"ok\":false}".to_string())
}

pub fn op_url_set_impl(href: &str, part: &str, value: &str) -> String {
    match std::panic::catch_unwind(|| url_set_inner(href, part, value)) {
        Ok(Some(v)) => v.to_string(),
        _ => match url::Url::parse(href) {
            Ok(u) => url_components(&u).to_string(),
            Err(_) => "{\"ok\":false}".to_string(),
        },
    }
}

pub fn op_url_resolve_impl(href: &str, base: &str) -> String {
    std::panic::catch_unwind(|| {
        let parsed = if base.is_empty() {
            url::Url::parse(href)
        } else {
            url::Url::parse(base).and_then(|b| b.join(href))
        };
        parsed.map(|u| u.as_str().to_string()).unwrap_or_default()
    })
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Encoding ops: text decoding, URL query encoding
// ---------------------------------------------------------------------------

pub fn op_encoding_for_label_impl(label: &str) -> String {
    obscura_net::label_name(label).unwrap_or_default()
}

pub fn op_text_decode_impl(label: &str, bytes: &[u8], fatal: bool, ignore_bom: bool) -> String {
    match obscura_net::decode_with_label(label, bytes, fatal, ignore_bom) {
        Some(s) => serde_json::json!({ "ok": true, "v": s }).to_string(),
        None => "{\"ok\":false}".to_string(),
    }
}

pub fn op_url_encode_query_impl(query: &str, label: &str, special: bool) -> String {
    obscura_net::url_encode_query(query, label, special).unwrap_or_else(|| query.to_string())
}

// ---------------------------------------------------------------------------
// Crypto ops: random bytes, digest, HMAC, AES (GCM/CBC/CTR), PBKDF2, HKDF
// Byte arrays cross the JS↔Rust boundary via base64 (see crypto_dispatch).
// ---------------------------------------------------------------------------

pub fn op_random_bytes_impl(len: u32) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; len as usize];
    getrandom::getrandom(&mut buf).map_err(|e| format!("getrandom failed: {e}"))?;
    Ok(buf)
}

pub fn op_subtle_digest_impl(algorithm: &str, data: &[u8]) -> Vec<u8> {
    use sha1::Digest as _;
    let alg = algorithm.to_ascii_uppercase();
    match alg.as_str() {
        "SHA-1" => sha1::Sha1::digest(data).to_vec(),
        "SHA-256" => sha2::Sha256::digest(data).to_vec(),
        "SHA-384" => sha2::Sha384::digest(data).to_vec(),
        "SHA-512" => sha2::Sha512::digest(data).to_vec(),
        "SHA-512/224" => sha2::Sha512_224::digest(data).to_vec(),
        "SHA-512/256" => sha2::Sha512_256::digest(data).to_vec(),
        _ => vec![],
    }
}

pub fn op_subtle_hmac_impl(hash: &str, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use hmac::{Hmac, Mac};
    macro_rules! run {
        ($d:ty) => {{
            let mut mac = Hmac::<$d>::new_from_slice(key).map_err(|e| e.to_string())?;
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    Ok(match hash {
        "SHA-1" => run!(sha1::Sha1),
        "SHA-256" => run!(sha2::Sha256),
        "SHA-384" => run!(sha2::Sha384),
        "SHA-512" => run!(sha2::Sha512),
        _ => return Err("unsupported HMAC hash".to_string()),
    })
}

pub fn op_subtle_aes_gcm_impl(encrypt: bool, key: &[u8], iv: &[u8], aad: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::aes::{Aes192, Aes256};
    use aes_gcm::{AesGcm, Nonce};
    type Aes192Gcm = AesGcm<Aes192, aes_gcm::aead::consts::U12>;
    type Aes256Gcm = AesGcm<Aes256, aes_gcm::aead::consts::U12>;

    if iv.len() != 12 { return Err("AES-GCM requires a 96-bit (12-byte) IV".into()); }
    let nonce = Nonce::from_slice(iv);
    macro_rules! run {
        ($ty:ty) => {{
            let cipher = <$ty>::new_from_slice(key).map_err(|e| e.to_string())?;
            if encrypt {
                cipher.encrypt(nonce, Payload { msg: data, aad }).map_err(|e| e.to_string())?
            } else {
                cipher.decrypt(nonce, Payload { msg: data, aad }).map_err(|_| "AES-GCM decryption failed: authentication tag mismatch".to_string())?
            }
        }};
    }
    Ok(match key.len() {
        16 => run!(aes_gcm::Aes128Gcm),
        24 => run!(Aes192Gcm),
        32 => run!(Aes256Gcm),
        _ => return Err("AES-GCM key must be 128, 192, or 256 bits".into()),
    })
}

pub fn op_subtle_aes_cbc_impl(encrypt: bool, key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use cbc::cipher::block_padding::Pkcs7;
    use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
    use cbc::{Decryptor, Encryptor};

    if iv.len() != 16 { return Err("AES-CBC requires a 16-byte IV".into()); }
    macro_rules! run {
        ($cipher:ty) => {{
            if encrypt {
                Encryptor::<$cipher>::new_from_slices(key, iv).map_err(|e| e.to_string())?
                    .encrypt_padded_vec_mut::<Pkcs7>(data)
            } else {
                Decryptor::<$cipher>::new_from_slices(key, iv).map_err(|e| e.to_string())?
                    .decrypt_padded_vec_mut::<Pkcs7>(data)
                    .map_err(|_| "AES-CBC decryption failed: invalid padding".to_string())?
            }
        }};
    }
    Ok(match key.len() {
        16 => run!(aes::Aes128),
        24 => run!(aes::Aes192),
        32 => run!(aes::Aes256),
        _ => return Err("AES-CBC key must be 128, 192, or 256 bits".into()),
    })
}

pub fn op_subtle_aes_ctr_impl(key: &[u8], counter: &[u8], counter_length: u32, data: &[u8]) -> Result<Vec<u8>, String> {
    use ctr::cipher::{KeyIvInit, StreamCipher};

    if counter.len() != 16 { return Err("AES-CTR requires a 16-byte counter block".into()); }
    let mut buf = data.to_vec();
    macro_rules! run {
        ($ty:ty) => {{
            <$ty>::new_from_slices(key, counter).map_err(|e| e.to_string())?.apply_keystream(&mut buf);
        }};
    }
    macro_rules! by_key {
        ($flavor:ident) => {
            match key.len() {
                16 => run!(ctr::$flavor<aes::Aes128>),
                24 => run!(ctr::$flavor<aes::Aes192>),
                32 => run!(ctr::$flavor<aes::Aes256>),
                _ => return Err("AES-CTR key must be 128, 192, or 256 bits".into()),
            }
        };
    }
    match counter_length {
        128 => by_key!(Ctr128BE),
        64 => by_key!(Ctr64BE),
        32 => by_key!(Ctr32BE),
        _ => return Err("AES-CTR supports counter lengths of 32, 64, or 128 bits".into()),
    }
    Ok(buf)
}

pub fn op_subtle_pbkdf2_impl(hash: &str, password: &[u8], salt: &[u8], iterations: u32, length: u32) -> Result<Vec<u8>, String> {
    use pbkdf2::pbkdf2_hmac;
    let mut dk = vec![0u8; length as usize];
    match hash {
        "SHA-1" => pbkdf2_hmac::<sha1::Sha1>(password, salt, iterations, &mut dk),
        "SHA-256" => pbkdf2_hmac::<sha2::Sha256>(password, salt, iterations, &mut dk),
        "SHA-384" => pbkdf2_hmac::<sha2::Sha384>(password, salt, iterations, &mut dk),
        "SHA-512" => pbkdf2_hmac::<sha2::Sha512>(password, salt, iterations, &mut dk),
        _ => return Err("unsupported PBKDF2 hash".into()),
    }
    Ok(dk)
}

pub fn op_subtle_hkdf_impl(hash: &str, ikm: &[u8], salt: &[u8], info: &[u8], length: u32) -> Result<Vec<u8>, String> {
    use hkdf::Hkdf;
    let mut okm = vec![0u8; length as usize];
    macro_rules! run {
        ($d:ty) => {
            Hkdf::<$d>::new(Some(salt), ikm).expand(info, &mut okm)
                .map_err(|_| "HKDF: requested key length is too long".to_string())?
        };
    }
    match hash {
        "SHA-1" => run!(sha1::Sha1),
        "SHA-256" => run!(sha2::Sha256),
        "SHA-384" => run!(sha2::Sha384),
        "SHA-512" => run!(sha2::Sha512),
        _ => return Err("unsupported HKDF hash".into()),
    }
    Ok(okm)
}

/// Dispatch function for all crypto ops. Takes a command name and JSON args
/// (array of base64-encoded byte arrays + other params), returns JSON with
/// either `{"result":"<base64>"}` or `{"error":"<msg>"}`.
fn crypto_dispatch(cmd: &str, args_json: &str) -> String {
    let args: Vec<serde_json::Value> = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"error": format!("bad args: {}", e)}).to_string(),
    };
    let b64_decode = |v: &serde_json::Value| -> Vec<u8> {
        v.as_str()
            .and_then(|s| BASE64.decode(s.as_bytes()).ok())
            .unwrap_or_default()
    };
    let result = match cmd {
        "digest" => {
            let alg = args.get(0).and_then(|v| v.as_str()).unwrap_or("");
            let data = b64_decode(args.get(1).unwrap_or(&serde_json::Value::Null));
            Ok(op_subtle_digest_impl(alg, &data))
        }
        "hmac" => {
            let hash = args.get(0).and_then(|v| v.as_str()).unwrap_or("");
            let key = b64_decode(args.get(1).unwrap_or(&serde_json::Value::Null));
            let data = b64_decode(args.get(2).unwrap_or(&serde_json::Value::Null));
            op_subtle_hmac_impl(hash, &key, &data)
        }
        "aes_gcm" => {
            let encrypt = args.get(0).and_then(|v| v.as_bool()).unwrap_or(true);
            let key = b64_decode(args.get(1).unwrap_or(&serde_json::Value::Null));
            let iv = b64_decode(args.get(2).unwrap_or(&serde_json::Value::Null));
            let aad = b64_decode(args.get(3).unwrap_or(&serde_json::Value::Null));
            let data = b64_decode(args.get(4).unwrap_or(&serde_json::Value::Null));
            op_subtle_aes_gcm_impl(encrypt, &key, &iv, &aad, &data)
        }
        "aes_cbc" => {
            let encrypt = args.get(0).and_then(|v| v.as_bool()).unwrap_or(true);
            let key = b64_decode(args.get(1).unwrap_or(&serde_json::Value::Null));
            let iv = b64_decode(args.get(2).unwrap_or(&serde_json::Value::Null));
            let data = b64_decode(args.get(3).unwrap_or(&serde_json::Value::Null));
            op_subtle_aes_cbc_impl(encrypt, &key, &iv, &data)
        }
        "aes_ctr" => {
            let key = b64_decode(args.get(0).unwrap_or(&serde_json::Value::Null));
            let counter = b64_decode(args.get(1).unwrap_or(&serde_json::Value::Null));
            let counter_length = args.get(2).and_then(|v| v.as_u64()).unwrap_or(128) as u32;
            let data = b64_decode(args.get(3).unwrap_or(&serde_json::Value::Null));
            op_subtle_aes_ctr_impl(&key, &counter, counter_length, &data)
        }
        "pbkdf2" => {
            let hash = args.get(0).and_then(|v| v.as_str()).unwrap_or("");
            let password = b64_decode(args.get(1).unwrap_or(&serde_json::Value::Null));
            let salt = b64_decode(args.get(2).unwrap_or(&serde_json::Value::Null));
            let iterations = args.get(3).and_then(|v| v.as_u64()).unwrap_or(1) as u32;
            let length = args.get(4).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            op_subtle_pbkdf2_impl(hash, &password, &salt, iterations, length)
        }
        "hkdf" => {
            let hash = args.get(0).and_then(|v| v.as_str()).unwrap_or("");
            let ikm = b64_decode(args.get(1).unwrap_or(&serde_json::Value::Null));
            let salt = b64_decode(args.get(2).unwrap_or(&serde_json::Value::Null));
            let info = b64_decode(args.get(3).unwrap_or(&serde_json::Value::Null));
            let length = args.get(4).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            op_subtle_hkdf_impl(hash, &ikm, &salt, &info, length)
        }
        _ => Err(format!("unknown crypto command: {}", cmd)),
    };
    match result {
        Ok(bytes) => serde_json::json!({"result": BASE64.encode(&bytes)}).to_string(),
        Err(e) => serde_json::json!({"error": e}).to_string(),
    }
}

// ---------------------------------------------------------------------------
// URL helpers: component serialization, mutation, host:port splitting
// ---------------------------------------------------------------------------

fn url_components(u: &url::Url) -> serde_json::Value {
    let port = u.port().map(|p| p.to_string()).unwrap_or_default();
    let hostname = u.host_str().unwrap_or("").to_string();
    let host = if hostname.is_empty() {
        String::new()
    } else if port.is_empty() {
        hostname.clone()
    } else {
        format!("{hostname}:{port}")
    };
    let search = match u.query() {
        Some(q) if !q.is_empty() => format!("?{q}"),
        _ => String::new(),
    };
    let hash = match u.fragment() {
        Some(f) if !f.is_empty() => format!("#{f}"),
        _ => String::new(),
    };
    serde_json::json!({
        "ok": true,
        "href": u.as_str(),
        "protocol": format!("{}:", u.scheme()),
        "username": u.username(),
        "password": u.password().unwrap_or(""),
        "host": host,
        "hostname": hostname,
        "port": port,
        "pathname": u.path(),
        "search": search,
        "hash": hash,
        "origin": u.origin().ascii_serialization(),
    })
}

fn url_set_inner(href: &str, part: &str, value: &str) -> Option<serde_json::Value> {
    let mut u = url::Url::parse(href).ok()?;
    match part {
        "href" => {
            let nu = url::Url::parse(value).ok()?;
            return Some(url_components(&nu));
        }
        "protocol" => { let _ = u.set_scheme(value.trim_end_matches(':')); }
        "username" => { let _ = u.set_username(value); }
        "password" => { let _ = u.set_password(if value.is_empty() { None } else { Some(value) }); }
        "host" => set_host_port(&mut u, value),
        "hostname" => { if !value.is_empty() { let _ = u.set_host(Some(value)); } }
        "port" => {
            if value.is_empty() { let _ = u.set_port(None); }
            else if let Ok(p) = value.parse::<u16>() { let _ = u.set_port(Some(p)); }
        }
        "pathname" => u.set_path(value),
        "search" => {
            let q = value.strip_prefix('?').unwrap_or(value);
            u.set_query(if q.is_empty() { None } else { Some(q) });
        }
        "hash" => {
            let f = value.strip_prefix('#').unwrap_or(value);
            u.set_fragment(if f.is_empty() { None } else { Some(f) });
        }
        _ => {}
    }
    Some(url_components(&u))
}

fn set_host_port(u: &mut url::Url, value: &str) {
    if value.starts_with('[') {
        if let Some(close) = value.find(']') {
            let host = &value[..=close];
            let rest = &value[close + 1..];
            if u.set_host(Some(host)).is_ok() {
                if let Some(p) = rest.strip_prefix(':') {
                    if let Ok(pn) = p.parse::<u16>() { let _ = u.set_port(Some(pn)); }
                }
            }
            return;
        }
    }
    if let Some(idx) = value.rfind(':') {
        let (h, p) = (&value[..idx], &value[idx + 1..]);
        if p.is_empty() || p.chars().all(|c| c.is_ascii_digit()) {
            if u.set_host(Some(h)).is_ok() {
                if p.is_empty() { let _ = u.set_port(None); }
                else if let Ok(pn) = p.parse::<u16>() { let _ = u.set_port(Some(pn)); }
            }
            return;
        }
    }
    let _ = u.set_host(Some(value));
}

// ---------------------------------------------------------------------------
// Fetch client cache + SSRF validation + URL pattern matching
// ---------------------------------------------------------------------------

static FETCH_CLIENT_CACHE: std::sync::OnceLock<
    std::sync::RwLock<std::collections::HashMap<String, reqwest::Client>>,
> = std::sync::OnceLock::new();

pub fn cached_request_client(proxy_url: Option<&str>) -> Result<reqwest::Client, String> {
    let key = proxy_url.unwrap_or("").to_string();
    let cache = FETCH_CLIENT_CACHE
        .get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    if let Ok(read) = cache.read() {
        if let Some(client) = read.get(&key) {
            return Ok(client.clone());
        }
    }
    let client = build_request_client(proxy_url)?;
    if let Ok(mut write) = cache.write() {
        write.entry(key).or_insert_with(|| client.clone());
    }
    Ok(client)
}

fn build_request_client(proxy_url: Option<&str>) -> Result<reqwest::Client, String> {
    let timeout_ms: u64 = std::env::var("OBSCURA_FETCH_TIMEOUT_MS")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(30_000);
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .dns_resolver(std::sync::Arc::new(obscura_net::SsrfGuardResolver::new(false)))
        .pool_idle_timeout(std::time::Duration::from_secs(300))
        .tcp_keepalive(std::time::Duration::from_secs(60));
    if let Some(proxy) = proxy_url {
        let p = reqwest::Proxy::all(proxy)
            .map_err(|e| format!("Invalid proxy '{}': {}", proxy, e))?;
        builder = builder.proxy(p);
    }
    builder.build().map_err(|e| format!("failed to build reqwest::Client: {}", e))
}

const FETCH_REDIRECT_LIMIT: usize = 10;

fn validate_fetch_url(url: &url::Url) -> Result<(), String> {
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" && scheme != "file" {
        return Err(format!("Forbidden URL scheme '{}' - only http, https, and file are allowed", scheme));
    }
    if scheme == "file" || obscura_net::env_allows_private_network() {
        return Ok(());
    }
    if let Some(host) = url.host() {
        match host {
            url::Host::Ipv4(ip) => {
                if obscura_net::is_forbidden_ip(std::net::IpAddr::V4(ip)) {
                    return Err(format!("Access to private/internal IP address {} is not allowed", ip));
                }
            }
            url::Host::Ipv6(ip) => {
                if obscura_net::is_forbidden_ip(std::net::IpAddr::V6(ip)) {
                    return Err(format!("Access to private/internal IPv6 address {} is not allowed", ip));
                }
            }
            url::Host::Domain(domain) => {
                let lower = domain.to_lowercase();
                if lower == "localhost" || lower.ends_with(".localhost") || lower == "127.0.0.1" || lower == "::1" {
                    return Err(format!("Access to localhost domain '{}' is not allowed", domain));
                }
            }
        }
    }
    Ok(())
}

/// Glob pattern matcher for URL blocking rules.
/// Supports `*` (any sequence) and `?` (single char) with backtracking.
fn glob_match(pattern: &str, url: &str) -> bool {
    // Simple glob: * matches any chars, ? matches one char
    // This is a simplified version; the original may use a more sophisticated matcher.
    let mut pi = pattern.chars().peekable();
    let mut ui = url.chars().peekable();
    let mut star_pi = None;
    let mut star_ui = ui.clone();
    loop {
        match (pi.peek(), ui.peek()) {
            (Some(&'*'), _) => {
                star_pi = Some(pi.clone());
                star_ui = ui.clone();
                pi.next();
            }
            (Some(&pc), Some(&uc)) if pc == uc || pc == '?' => {
                pi.next();
                ui.next();
            }
            (Some(_), Some(_)) => {
                if let Some(mut spi) = star_pi.take() {
                    spi.next();
                    pi = spi.clone();
                    star_ui.next();
                    ui = star_ui.clone();
                    star_pi = Some(spi);
                } else {
                    return false;
                }
            }
            (None, None) => return true,
            (None, Some(_)) => {
                if star_pi.is_some() {
                    star_ui.next();
                    ui = star_ui.clone();
                } else {
                    return false;
                }
            }
            (Some(_), None) => {
                if pi.peek() == Some(&'*') {
                    pi.next();
                } else {
                    return false;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Web search: DuckDuckGo HTML endpoint → parsed results
// ---------------------------------------------------------------------------

/// Web search implementation using DuckDuckGo HTML endpoint.
/// Returns JSON array of {title, url, snippet}.
pub async fn op_web_search_impl(query: &str) -> Result<String, String> {
    let search_url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    let client = cached_request_client(None)?;
    let resp = client
        .get(&search_url)
        .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {}", e))?;

    let html = resp.text().await.map_err(|e| format!("Failed to read search response: {}", e))?;
    let results = parse_ddg_results(&html);
    Ok(serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string()))
}

/// Parse DuckDuckGo HTML results page into search results.
/// DDG structure: `<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=ENCODED">Title</a>`
/// followed by `<a class="result__snippet" href="...">Snippet</a>`.
fn parse_ddg_results(html: &str) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    let mut rest = html;

    while let Some(a_start) = rest.find("class=\"result__a\"") {
        // In DDG HTML, href comes AFTER class="result__a", so search forward.
        let after_class = &rest[a_start + 16..]; // skip past class="result__a"
        let href_pos = match after_class.find("href=\"") {
            Some(pos) => pos,
            None => { rest = after_class; continue; }
        };
        // Make sure this href is within the same <a> tag (not too far away)
        if href_pos > 50 {
            rest = after_class;
            continue;
        }
        let href_start_rel = href_pos + 6;
        let after_href = &after_class[href_start_rel..];
        let href_end = match after_href.find('"') {
            Some(pos) => pos,
            None => { rest = after_class; continue; }
        };
        let raw_href = &after_href[..href_end];
        let url = extract_ddg_url(raw_href);

        // Find the '>' that closes this <a> opening tag (after href)
        let tag_close_rel = match after_href[href_end..].find('>') {
            Some(pos) => pos,
            None => { rest = after_href; continue; }
        };
        let inner = &after_href[href_end + tag_close_rel + 1..];
        let title_end = match inner.find("</a>") {
            Some(pos) => pos,
            None => { rest = after_href; continue; }
        };
        let title_raw = &inner[..title_end];
        let title = strip_html_tags(title_raw).trim().to_string();

        let snippet_rest = &inner[title_end..];
        let snippet = if let Some(snip_start) = snippet_rest.find("class=\"result__snippet\"") {
            let after_snip = &snippet_rest[snip_start..];
            if let Some(s_tag_end) = after_snip.find('>') {
                let after_snip_tag = &after_snip[s_tag_end + 1..];
                if let Some(s_close) = after_snip_tag.find("</a>") {
                    strip_html_tags(&after_snip_tag[..s_close]).trim().to_string()
                } else if let Some(s_close2) = after_snip_tag.find("</span>") {
                    strip_html_tags(&after_snip_tag[..s_close2]).trim().to_string()
                } else { String::new() }
            } else { String::new() }
        } else { String::new() };

        if !title.is_empty() && !url.is_empty() {
            results.push(serde_json::json!({
                "title": title,
                "url": url,
                "snippet": snippet,
            }));
        }
        rest = snippet_rest;
    }
    results
}

/// Extract the actual URL from a DuckDuckGo redirect link.
fn extract_ddg_url(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let after = &href[pos + 5..];
        let end = after.find('&').unwrap_or(after.len());
        let encoded = &after[..end];
        if let Ok(decoded) = urlencoding::decode(encoded) {
            return decoded.to_string();
        }
        return encoded.to_string();
    }
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    href.to_string()
}

/// Strip HTML tags from a string
fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

// ---------------------------------------------------------------------------
// Bing HTML search provider (source #2 for metasearch)
// ---------------------------------------------------------------------------

/// Search Bing's HTML endpoint and return parsed results.
pub async fn op_bing_search_impl(query: &str) -> Result<String, String> {
    let search_url = format!(
        "https://www.bing.com/search?q={}&count=20",
        urlencoding::encode(query)
    );

    let client = cached_request_client(None)?;
    let resp = client
        .get(&search_url)
        .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36")
        .send()
        .await
        .map_err(|e| format!("Bing search request failed: {}", e))?;

    let html = resp.text().await.map_err(|e| format!("Failed to read Bing response: {}", e))?;
    let results = parse_bing_results(&html);
    Ok(serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string()))
}

/// Parse Bing HTML results page.
/// Bing structure: `<li class="b_algo">` containing `<h2><a href="URL">Title</a></h2>` and `<p>snippet</p>`
fn parse_bing_results(html: &str) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    let mut rest = html;

    while let Some(block_start) = rest.find("class=\"b_algo\"") {
        // Find the next b_algo block or end
        let block_end = rest[block_start + 14..]
            .find("class=\"b_algo\"")
            .map(|p| block_start + 14 + p)
            .unwrap_or(rest.len());

        let block = &rest[block_start..block_end];

        // Extract URL from first <a href="..."> in the block
        let url = if let Some(href_start) = block.find("href=\"") {
            let after = &block[href_start + 6..];
            let end = after.find('"').unwrap_or(after.len());
            &after[..end]
        } else {
            rest = &rest[block_start + 14..];
            continue;
        };

        // Extract title: text between first <a ...>TITLE</a>
        let title = if let Some(a_tag_end) = block.find("href=\"") {
            let after_href = &block[a_tag_end..];
            if let Some(close) = after_href.find('>') {
                let inner = &after_href[close + 1..];
                if let Some(end_a) = inner.find("</a>") {
                    strip_html_tags(&inner[..end_a]).trim().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Extract snippet: text in <p> or class="b_caption"
        let snippet = if let Some(p_start) = block.find("<p>") {
            let after = &block[p_start + 3..];
            let end = after.find("</p>").unwrap_or(after.len());
            strip_html_tags(&after[..end]).trim().to_string()
        } else if let Some(cap_start) = block.find("b_caption") {
            let after = &block[cap_start..];
            if let Some(p_open) = after.find("<p") {
                let after_p = &after[p_open..];
                if let Some(gt) = after_p.find('>') {
                    let inner = &after_p[gt + 1..];
                    let end = inner.find("</p>").unwrap_or(inner.len());
                    strip_html_tags(&inner[..end]).trim().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
            results.push(serde_json::json!({
                "title": title,
                "url": url,
                "snippet": snippet,
            }));
        }

        rest = &rest[block_start + 14..];
    }

    results
}

// ---------------------------------------------------------------------------
// Wikipedia API search provider (source #3 for metasearch)
// ---------------------------------------------------------------------------

/// Search Wikipedia's API endpoint for high-quality encyclopedic results.
pub async fn op_wikipedia_search_impl(query: &str) -> Result<String, String> {
    let search_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=5&srprop=snippet",
        urlencoding::encode(query)
    );

    let client = cached_request_client(None)?;
    let resp = client
        .get(&search_url)
        .header("User-Agent", "ObscuraSearch/1.0 (Android/Termux)")
        .send()
        .await
        .map_err(|e| format!("Wikipedia search request failed: {}", e))?;

    let body = resp.text().await.map_err(|e| format!("Failed to read Wikipedia response: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse Wikipedia JSON: {}", e))?;

    let mut results = Vec::new();
    if let Some(search_items) = json.get("query").and_then(|q| q.get("search")).and_then(|s| s.as_array()) {
        for item in search_items {
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let snippet_raw = item.get("snippet").and_then(|s| s.as_str()).unwrap_or("");
            let pageid = item.get("pageid").and_then(|p| p.as_i64()).unwrap_or(0);

            let url = format!("https://en.wikipedia.org/?curid={}", pageid);
            let snippet = strip_html_tags(snippet_raw).trim().to_string();

            if !title.is_empty() {
                results.push(serde_json::json!({
                    "title": title,
                    "url": url,
                    "snippet": snippet,
                }));
            }
        }
    }

    Ok(serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string()))
}

// ---------------------------------------------------------------------------
// Metasearch: query multiple providers in parallel, aggregate, dedup, rank
// ---------------------------------------------------------------------------

/// A single search result with metadata for ranking.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
    pub rank_score: f64,
}

/// Query DuckDuckGo, Bing, and Wikipedia in parallel, then dedup and rank.
/// Targets 8–15 high-quality results.
pub async fn metasearch(query: &str) -> Result<String, String> {
    let (ddg_res, bing_res, wiki_res) = tokio::join!(
        op_web_search_impl(query),
        op_bing_search_impl(query),
        op_wikipedia_search_impl(query),
    );

    let mut all_results: Vec<SearchResult> = Vec::new();

    // DDG results
    if let Ok(json) = ddg_res {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            for (i, r) in arr.iter().enumerate() {
                all_results.push(SearchResult {
                    title: r.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    snippet: r.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    source: "duckduckgo".to_string(),
                    rank_score: 0.0,
                });
                let _ = i;
            }
        }
    }

    // Bing results
    if let Ok(json) = bing_res {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            for r in arr.iter() {
                all_results.push(SearchResult {
                    title: r.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    snippet: r.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    source: "bing".to_string(),
                    rank_score: 0.0,
                });
            }
        }
    }

    // Wikipedia results
    if let Ok(json) = wiki_res {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            for r in arr.iter() {
                all_results.push(SearchResult {
                    title: r.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    snippet: r.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    source: "wikipedia".to_string(),
                    rank_score: 0.0,
                });
            }
        }
    }

    // Deduplicate by normalized URL
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deduped: Vec<SearchResult> = Vec::new();
    for r in all_results {
        let normalized = normalize_url(&r.url);
        if !normalized.is_empty() && seen.insert(normalized) {
            deduped.push(r);
        }
    }

    // Rank: multi-source boost + snippet quality
    for r in &mut deduped {
        let mut score = 0.0;

        // Source priority: Wikipedia is high-trust, DDG/Bing are general
        score += match r.source.as_str() {
            "wikipedia" => 15.0,
            "duckduckgo" => 10.0,
            "bing" => 8.0,
            _ => 5.0,
        };

        // Snippet length bonus (longer = more info)
        let snip_len = r.snippet.len().min(300) as f64;
        score += snip_len / 30.0;

        // Title relevance: contains query words
        let query_lower = query.to_lowercase();
        let title_lower = r.title.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();
        for word in &query_words {
            if title_lower.contains(word) {
                score += 2.0;
            }
        }

        r.rank_score = score;
    }

    // Sort by score descending
    deduped.sort_by(|a, b| b.rank_score.partial_cmp(&a.rank_score).unwrap_or(std::cmp::Ordering::Equal));

    // Cap at 15 results
    deduped.truncate(15);

    // Serialize
    let results_json: Vec<serde_json::Value> = deduped
        .iter()
        .map(|r| {
            serde_json::json!({
                "title": r.title,
                "url": r.url,
                "snippet": r.snippet,
                "source": r.source,
                "score": r.rank_score,
            })
        })
        .collect();

    Ok(serde_json::to_string(&results_json).unwrap_or_else(|_| "[]".to_string()))
}

/// Normalize a URL for deduplication: lowercase host, strip trailing slash, strip fragments.
fn normalize_url(url: &str) -> String {
    let lower = url.to_lowercase();
    let without_frag = lower.split('#').next().unwrap_or(&lower);
    let trimmed = without_frag.trim_end_matches('/');
    if let Ok(parsed) = url::Url::parse(trimmed) {
        format!("{}{}{}", parsed.host_str().unwrap_or(""), parsed.path(), parsed.query().unwrap_or(""))
    } else {
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------------
// op_fetch_url: full HTTP fetch with CORS, cookies, redirects, interception
// ---------------------------------------------------------------------------

/// Full op_fetch_url implementation. Returns a JSON string with the response.
pub async fn op_fetch_url_impl(
    state: &SharedState,
    url: String,
    method: String,
    headers_json: String,
    body: String,
    origin: String,
    mode: String,
) -> Result<String, String> {
    tracing::debug!("op_fetch_url called: {} {}", method, url);

    if let Ok(parsed_url) = url::Url::parse(&url) {
        if let Err(e) = validate_fetch_url(&parsed_url) {
            return Ok(serde_json::json!({
                "status": 0, "body": "", "url": url, "headers": {},
                "blocked": true, "error": e,
            }).to_string());
        }
    }

    let (cookie_jar, in_flight, intercept_tx, proxy_url, http_client) = {
        let mut gs = state.lock().unwrap();
        for pattern in &gs.blocked_urls {
            if pattern == "*" || url.contains(pattern) || glob_match(pattern, &url) {
                return Ok(serde_json::json!({
                    "status": 0, "body": "", "url": url, "headers": {}, "blocked": true,
                }).to_string());
            }
        }
        gs.fetched_urls.push(url.clone());
        let jar = gs.cookie_jar.clone();
        let in_flight = gs.http_client.as_ref().map(|c| c.in_flight.clone());
        let proxy_url = gs.http_client.as_ref().and_then(|c| c.proxy_url().map(|s| s.to_string()));
        let itx = if gs.intercept_enabled {
            let counter = gs.intercept_counter + 1;
            gs.intercept_tx.clone().map(|tx| (tx, format!("intercept-{}", counter)))
        } else {
            None
        };
        (jar, in_flight, itx, proxy_url, gs.http_client.clone())
    };

    let mut override_url: Option<String> = None;
    let mut override_method: Option<String> = None;
    let mut override_headers: Option<HashMap<String, String>> = None;
    let mut override_body: Option<String> = None;

    if let Some((tx, request_id)) = intercept_tx {
        let custom_headers: HashMap<String, String> = serde_json::from_str(&headers_json).unwrap_or_default();
        let (resolve_tx, resolve_rx) = tokio::sync::oneshot::channel();
        let intercepted = InterceptedRequest {
            request_id: request_id.clone(),
            url: url.clone(),
            method: method.clone(),
            headers: custom_headers.clone(),
            resource_type: "Fetch".to_string(),
            resolver: resolve_tx,
        };
        if tx.send(intercepted).is_ok() {
            match resolve_rx.await {
                Ok(InterceptResolution::Fulfill { status, headers: h, body: b }) => {
                    return Ok(serde_json::json!({
                        "status": status, "body": b, "url": url, "headers": h,
                    }).to_string());
                }
                Ok(InterceptResolution::Fail { reason }) => {
                    return Ok(serde_json::json!({
                        "status": 0, "body": "", "url": url, "headers": {},
                        "blocked": true, "error": reason,
                    }).to_string());
                }
                Ok(InterceptResolution::Continue { url, method, headers, body }) => {
                    override_url = url;
                    override_method = method;
                    override_headers = headers;
                    override_body = body;
                }
                Err(_) => {}
            }
        }
    }

    let url = if let Some(new_url) = override_url {
        if let Ok(parsed) = url::Url::parse(&new_url) {
            if let Err(reason) = validate_fetch_url(&parsed) {
                return Ok(serde_json::json!({
                    "status": 0, "body": "", "url": new_url, "headers": {},
                    "blocked": true,
                    "error": format!("Intercept rewrite to forbidden URL blocked: {}", reason),
                }).to_string());
            }
        }
        new_url
    } else { url };
    let method = override_method.unwrap_or(method);
    let body = override_body.unwrap_or(body);

    let client = cached_request_client(proxy_url.as_deref())?;

    let request_origin = url::Url::parse(&url).ok().map(|u| {
        let host = u.host_str().unwrap_or("");
        match u.port() {
            Some(p) => format!("{}://{}:{}", u.scheme(), host, p),
            None => format!("{}://{}", u.scheme(), host),
        }
    }).unwrap_or_default();
    let page_origin = if origin.is_empty() { request_origin.clone() } else { origin.clone() };
    let is_cross_origin = !page_origin.is_empty() && request_origin != page_origin;

    let req_method: reqwest::Method = method.parse().unwrap_or(reqwest::Method::GET);
    let custom_headers: HashMap<String, String> =
        override_headers.unwrap_or_else(|| serde_json::from_str(&headers_json).unwrap_or_default());

    if let Some(ref hc) = http_client {
        let cbs = hc.on_request.read().await;
        if !cbs.is_empty() {
            if let Ok(parsed) = url::Url::parse(&url) {
                let info = RequestInfo {
                    url: parsed, method: method.clone(), headers: custom_headers.clone(),
                    resource_type: ResourceType::Fetch,
                };
                for cb in cbs.iter() { cb(&info); }
            }
        }
    }

    #[cfg(feature = "stealth")]
    {
        let stealth = state.lock().unwrap().stealth_client.clone();
        if let Some(stealth) = stealth {
            return stealth_fetch_all(stealth, url.clone(), req_method.as_str().to_string(),
                custom_headers.clone(), body.clone(), page_origin.clone(),
                is_cross_origin, mode.clone(), http_client.clone()).await;
        }
    }

    let needs_preflight = is_cross_origin
        && mode == "cors"
        && (req_method != reqwest::Method::GET
            && req_method != reqwest::Method::HEAD
            && req_method != reqwest::Method::POST
            || custom_headers.keys().any(|k| {
                let kl = k.to_lowercase();
                kl != "accept" && kl != "accept-language" && kl != "content-language"
                    && kl != "content-type"
            }));

    if needs_preflight {
        let preflight = client.request(reqwest::Method::OPTIONS, &url)
            .header("Origin", &page_origin)
            .header("Access-Control-Request-Method", method.as_str())
            .header("Access-Control-Request-Headers",
                custom_headers.keys().cloned().collect::<Vec<_>>().join(", "))
            .send().await
            .map_err(|e| format!("CORS preflight failed: {}", e))?;

        let allowed_origin = preflight.headers().get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()).unwrap_or("");
        if allowed_origin != "*" && allowed_origin != page_origin {
            return Err(format!("CORS preflight: Origin '{}' not allowed by Access-Control-Allow-Origin '{}'", page_origin, allowed_origin));
        }
    }

    let mut current_url = url.clone();
    let mut current_method = req_method;
    let mut current_body = body;
    let mut redirects_followed: usize = 0;
    let response = loop {
        let mut req = client.request(current_method.clone(), &current_url);
        if is_cross_origin { req = req.header("Origin", &page_origin); }
        if !is_cross_origin {
            if let Some(ref jar) = cookie_jar {
                if let Ok(parsed_url) = url::Url::parse(&current_url) {
                    let cookie_header = jar.get_cookie_header(&parsed_url);
                    if !cookie_header.is_empty() { req = req.header("Cookie", &cookie_header); }
                }
            }
        }
        if !custom_headers.keys().any(|k| k.eq_ignore_ascii_case("user-agent")) {
            req = req.header("User-Agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36");
        }
        for (k, v) in &custom_headers { req = req.header(k.as_str(), v.as_str()); }
        if !current_body.is_empty() { req = req.body(current_body.clone()); }
        if let Some(ref counter) = in_flight { counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
        let resp = req.send().await.map_err(|e| {
            if let Some(ref counter) = in_flight { counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed); }
            e.to_string()
        })?;
        if let Some(ref counter) = in_flight { counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed); }
        if let Some(ref jar) = cookie_jar {
            if let Ok(parsed_url) = url::Url::parse(&current_url) {
                for val in resp.headers().get_all(reqwest::header::SET_COOKIE) {
                    if let Ok(s) = val.to_str() { jar.set_cookie(s, &parsed_url); }
                }
            }
        }
        if !resp.status().is_redirection() { break resp; }
        let location_header = resp.headers().get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok()).map(str::to_string);
        let Some(location) = location_header else { break resp; };
        let base = match url::Url::parse(&current_url) { Ok(b) => b, Err(_) => break resp };
        let next_url = match base.join(&location) { Ok(u) => u, Err(_) => break resp };
        if let Err(reason) = validate_fetch_url(&next_url) {
            return Ok(serde_json::json!({
                "status": 0, "body": "", "url": next_url.to_string(), "headers": {},
                "blocked": true, "error": format!("Redirect to forbidden URL blocked: {}", reason),
            }).to_string());
        }
        redirects_followed += 1;
        if redirects_followed > FETCH_REDIRECT_LIMIT {
            return Ok(serde_json::json!({
                "status": 0, "body": "", "url": next_url.to_string(), "headers": {},
                "blocked": true, "error": format!("Too many redirects (>{})", FETCH_REDIRECT_LIMIT),
            }).to_string());
        }
        let status_code = resp.status().as_u16();
        if status_code == 301 || status_code == 302 || status_code == 303 {
            current_method = reqwest::Method::GET;
            current_body.clear();
        }
        current_url = next_url.to_string();
    };

    let status = response.status().as_u16();
    let resp_headers: HashMap<String, String> = response.headers().iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();

    if is_cross_origin && mode == "cors" {
        let allowed = resp_headers.get("access-control-allow-origin").map(|s| s.as_str()).unwrap_or("");
        if allowed != "*" && allowed != page_origin {
            return Ok(serde_json::json!({
                "status": 0, "body": "", "url": url, "headers": {},
                "corsBlocked": true,
                "corsError": format!("CORS error: Origin '{}' not in Access-Control-Allow-Origin '{}'", page_origin, allowed),
            }).to_string());
        }
    }

    let resp_bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let resp_body = String::from_utf8_lossy(&resp_bytes).to_string();
    let resp_body_base64 = BASE64.encode(&resp_bytes);

    if let Some(ref hc) = http_client {
        let cbs = hc.on_response.read().await;
        if !cbs.is_empty() {
            let resp = fetch_response(&url, status, resp_headers.clone(), resp_bytes.to_vec());
            let info = RequestInfo {
                url: resp.url.clone(), method: method.clone(), headers: resp_headers.clone(),
                resource_type: ResourceType::Fetch,
            };
            for cb in cbs.iter() { cb(&info, &resp); }
        }
    }

    let response_request_id = {
        let mut gs = state.lock().unwrap();
        gs.network_response_body_counter += 1;
        let request_id = format!("fetch-{}", gs.network_response_body_counter);
        let max_entries = response_body_entry_limit();
        let max_bytes = response_body_byte_limit();
        if max_entries > 0 && max_bytes > 0 && resp_bytes.len() <= max_bytes {
            gs.network_response_bodies.insert(request_id.clone(), StoredNetworkResponseBody {
                body: resp_body.clone(), base64_encoded: false,
            });
            gs.network_response_body_order.push_back(request_id.clone());
            while gs.network_response_body_order.len() > max_entries {
                if let Some(oldest) = gs.network_response_body_order.pop_front() {
                    gs.network_response_bodies.remove(&oldest);
                }
            }
        }
        request_id
    };

    Ok(serde_json::json!({
        "status": status, "body": resp_body, "bodyBase64": resp_body_base64,
        "requestId": response_request_id, "url": url, "headers": resp_headers,
    }).to_string())
}

fn response_body_entry_limit() -> usize {
    std::env::var("OBSCURA_NETWORK_BODY_BUFFER_ENTRIES").ok()
        .and_then(|v| v.parse().ok()).unwrap_or(128)
}

fn response_body_byte_limit() -> usize {
    std::env::var("OBSCURA_NETWORK_BODY_BUFFER_BYTES").ok()
        .and_then(|v| v.parse().ok()).unwrap_or(2 * 1024 * 1024)
}

fn fetch_response(url: &str, status: u16, headers: HashMap<String, String>, body: Vec<u8>) -> Response {
    Response {
        url: url::Url::parse(url).unwrap_or_else(|_| url::Url::parse("http://0.0.0.0/").unwrap()),
        status, headers, body, redirected_from: Vec::new(),
    }
}

#[cfg(feature = "stealth")]
async fn stealth_fetch_all(
    stealth: Arc<StealthHttpClient>,
    url: String,
    method: String,
    custom_headers: HashMap<String, String>,
    body: String,
    page_origin: String,
    is_cross_origin: bool,
    mode: String,
    http_client: Option<Arc<ObscuraHttpClient>>,
) -> Result<String, String> {
    let mut current_url = url.clone();
    let mut current_method = method;
    let mut current_body = body;
    let mut redirects_followed: usize = 0;

    let (status, resp_headers, resp_bytes): (u16, HashMap<String, String>, Vec<u8>) = loop {
        let parsed_current = match url::Url::parse(&current_url) {
            Ok(u) => u,
            Err(_) => return Ok(serde_json::json!({"status": 0, "body": "", "url": current_url, "headers": {}}).to_string()),
        };
        let mut req_headers: HashMap<String, String> = HashMap::new();
        if is_cross_origin { req_headers.insert("origin".to_string(), page_origin.clone()); }
        for (k, v) in &custom_headers { req_headers.insert(k.to_lowercase(), v.clone()); }
        let r = stealth.send_single(&current_method, &parsed_current, &req_headers, &current_body).await
            .map_err(|e| e.to_string())?;
        if !(300..400).contains(&r.status) { break (r.status, r.headers, r.body); }
        let Some(location) = r.headers.get("location").cloned() else { break (r.status, r.headers, r.body); };
        let next_url = match parsed_current.join(&location) { Ok(u) => u, Err(_) => break (r.status, r.headers, r.body) };
        if let Err(reason) = validate_fetch_url(&next_url) {
            return Ok(serde_json::json!({
                "status": 0, "body": "", "url": next_url.to_string(), "headers": {},
                "blocked": true, "error": format!("Redirect to forbidden URL blocked: {}", reason),
            }).to_string());
        }
        redirects_followed += 1;
        if redirects_followed > FETCH_REDIRECT_LIMIT {
            return Ok(serde_json::json!({
                "status": 0, "body": "", "url": next_url.to_string(), "headers": {},
                "blocked": true, "error": format!("Too many redirects (>{})", FETCH_REDIRECT_LIMIT),
            }).to_string());
        }
        if r.status == 301 || r.status == 302 || r.status == 303 {
            current_method = "GET".to_string();
            current_body.clear();
        }
        current_url = next_url.to_string();
    };

    if is_cross_origin && mode == "cors" {
        let allowed = resp_headers.get("access-control-allow-origin").map(|s| s.as_str()).unwrap_or("");
        if allowed != "*" && allowed != page_origin {
            return Ok(serde_json::json!({
                "status": 0, "body": "", "url": url, "headers": {},
                "corsBlocked": true,
                "corsError": format!("CORS error: Origin '{}' not in Access-Control-Allow-Origin '{}'", page_origin, allowed),
            }).to_string());
        }
    }

    let resp_body = String::from_utf8_lossy(&resp_bytes).to_string();
    let resp_body_base64 = BASE64.encode(&resp_bytes);
    if let Some(ref hc) = http_client {
        let cbs = hc.on_response.read().await;
        if !cbs.is_empty() {
            let resp = fetch_response(&url, status, resp_headers.clone(), resp_bytes.clone());
            let info = RequestInfo {
                url: resp.url.clone(), method: current_method.clone(), headers: resp_headers.clone(),
                resource_type: ResourceType::Fetch,
            };
            for cb in cbs.iter() { cb(&info, &resp); }
        }
    }

    Ok(serde_json::json!({
        "status": status, "body": resp_body, "bodyBase64": resp_body_base64,
        "url": url, "headers": resp_headers,
    }).to_string())
}

// ---------------------------------------------------------------------------
// install_ops: bridge that installs all Rust ops onto globalThis.Deno.core.ops
// ---------------------------------------------------------------------------

// Crypto/byte ops use a base64 bridge to avoid rquickjs Value lifetime issues:
// JS wrappers convert TypedArray ↔ base64, then call the Rust dispatch function.
// Async ops (op_fetch_url, op_web_search) use a JS Promise + queue pattern:
// JS pushes to a queue array, the runtime's pump_jobs_and_async drains it and
// spawns tokio tasks, then resolve_completed_async calls the JS resolvers.

/// Install `globalThis.Deno.core.ops = { ...all ops... }` into the given context.
pub fn install_ops(ctx: &Ctx, state: SharedState) -> rquickjs::Result<()> {
    let global = ctx.globals();

    let deno = Object::new(ctx.clone())?;
    let core = Object::new(ctx.clone())?;
    let ops = Object::new(ctx.clone())?;

    // ---- STATE ops ----
    {
        let st = state.clone();
        ops.set("op_dom", Func::from(move |cmd: String, arg1: String, arg2: String| {
            op_dom_impl(&st, &cmd, &arg1, &arg2)
        }))?;
    }
    {
        ops.set("op_console_msg", Func::from(|level: String, msg: String| {
            op_console_msg_impl(&level, &msg);
        }))?;
    }
    {
        let st = state.clone();
        ops.set("op_get_cookies", Func::from(move || {
            op_get_cookies_impl(&st)
        }))?;
    }
    {
        let st = state.clone();
        ops.set("op_set_cookie", Func::from(move |cookie_str: String| {
            op_set_cookie_impl(&st, &cookie_str);
        }))?;
    }
    {
        let st = state.clone();
        ops.set("op_navigate", Func::from(move |url: String, method: String, body: String| {
            op_navigate_impl(&st, &url, &method, &body);
        }))?;
    }
    {
        let st = state.clone();
        ops.set("op_binding_called", Func::from(move |name: String, payload: String| {
            op_binding_called_impl(&st, &name, &payload);
        }))?;
    }

    // ---- PURE ops ----
    ops.set("op_url_parse", Func::from(|href: String, base: String| {
        op_url_parse_impl(&href, &base)
    }))?;
    ops.set("op_url_set", Func::from(|href: String, part: String, value: String| {
        op_url_set_impl(&href, &part, &value)
    }))?;
    ops.set("op_url_resolve", Func::from(|href: String, base: String| {
        op_url_resolve_impl(&href, &base)
    }))?;
    ops.set("op_encoding_for_label", Func::from(|label: String| {
        op_encoding_for_label_impl(&label)
    }))?;
    ops.set("op_url_encode_query", Func::from(|query: String, label: String, special: bool| {
        op_url_encode_query_impl(&query, &label, special)
    }))?;
    // ---- CRYPTO + byte ops ----
    ops.set("op_crypto_dispatch", Func::from(|cmd: String, args_json: String| -> String {
        crypto_dispatch(&cmd, &args_json)
    }))?;

    // op_random_bytes: returns base64-encoded bytes
    ops.set("op_random_bytes_b64", Func::from(|len: u32| -> String {
        match op_random_bytes_impl(len) {
            Ok(bytes) => BASE64.encode(&bytes),
            Err(e) => format!("ERROR:{}", e),
        }
    }))?;

    // op_text_decode_b64: takes label, base64 bytes, fatal, ignore_bom
    ops.set("op_text_decode_b64", Func::from(|label: String, data_b64: String, fatal: bool, ignore_bom: bool| -> String {
        match BASE64.decode(data_b64.as_bytes()) {
            Ok(bytes) => op_text_decode_impl(&label, &bytes, fatal, ignore_bom),
            Err(_) => "{\"ok\":false}".to_string(),
        }
    }))?;

    // Install the ops object as Deno.core.ops FIRST, so JS wrappers can reference it.
    core.set("ops", ops)?;
    deno.set("core", core)?;
    global.set("Deno", deno)?;

    // Now install JS wrappers that convert TypedArray <-> base64 for all crypto ops.
    // These reference globalThis.Deno.core.ops which is now available.
    ctx.eval::<(), _>(r#"
        (function() {
            function toB64(ta) {
                if (!ta) return "";
                var a = new Uint8Array(ta.buffer || ta, ta.byteOffset || 0, ta.byteLength || ta.length);
                var s = "";
                for (var i = 0; i < a.length; i++) s += String.fromCharCode(a[i]);
                return btoa(s);
            }
            function fromB64(b64) {
                var s = atob(b64);
                var a = new Uint8Array(s.length);
                for (var i = 0; i < s.length; i++) a[i] = s.charCodeAt(i);
                return a;
            }

            var ops = globalThis.Deno.core.ops;
            var d = Deno.core.ops.op_crypto_dispatch;

            ops.op_random_bytes = function(len) {
                var b64 = Deno.core.ops.op_random_bytes_b64(len);
                if (b64.startsWith("ERROR:")) throw new Error(b64.slice(6));
                return fromB64(b64);
            };

            ops.op_subtle_digest = function(algorithm, data) {
                var r = d("digest", JSON.stringify([algorithm, toB64(data)]));
                var p = JSON.parse(r);
                if (p.error) throw new Error(p.error);
                return fromB64(p.result);
            };

            ops.op_subtle_hmac = function(hash, key, data) {
                var r = d("hmac", JSON.stringify([hash, toB64(key), toB64(data)]));
                var p = JSON.parse(r);
                if (p.error) throw new Error(p.error);
                return fromB64(p.result);
            };

            ops.op_subtle_aes_gcm = function(encrypt, key, iv, aad, data) {
                var r = d("aes_gcm", JSON.stringify([encrypt, toB64(key), toB64(iv), toB64(aad), toB64(data)]));
                var p = JSON.parse(r);
                if (p.error) throw new Error(p.error);
                return fromB64(p.result);
            };

            ops.op_subtle_aes_cbc = function(encrypt, key, iv, data) {
                var r = d("aes_cbc", JSON.stringify([encrypt, toB64(key), toB64(iv), toB64(data)]));
                var p = JSON.parse(r);
                if (p.error) throw new Error(p.error);
                return fromB64(p.result);
            };

            ops.op_subtle_aes_ctr = function(key, counter, counter_length, data) {
                var r = d("aes_ctr", JSON.stringify([toB64(key), toB64(counter), counter_length, toB64(data)]));
                var p = JSON.parse(r);
                if (p.error) throw new Error(p.error);
                return fromB64(p.result);
            };

            ops.op_subtle_pbkdf2 = function(hash, password, salt, iterations, length) {
                var r = d("pbkdf2", JSON.stringify([hash, toB64(password), toB64(salt), iterations, length]));
                var p = JSON.parse(r);
                if (p.error) throw new Error(p.error);
                return fromB64(p.result);
            };

            ops.op_subtle_hkdf = function(hash, ikm, salt, info, length) {
                var r = d("hkdf", JSON.stringify([hash, toB64(ikm), toB64(salt), toB64(info), length]));
                var p = JSON.parse(r);
                if (p.error) throw new Error(p.error);
                return fromB64(p.result);
            };

            ops.op_text_decode = function(label, data, fatal, ignore_bom) {
                return Deno.core.ops.op_text_decode_b64(label, toB64(data), fatal, ignore_bom);
            };
        })();
    "#)?;

    // ---- ASYNC ops (op_fetch_url, op_sleep, op_web_search) ----
    {
        ctx.eval::<(), _>(
            r#"globalThis.__obscura_fetch_queue = [];
            globalThis.__obscura_fetch_id = 0;
            globalThis.__obscura_fetch_resolvers = {};
            globalThis.Deno.core.ops.op_fetch_url = function(url, method, headers, body, origin, mode) {
                var id = ++globalThis.__obscura_fetch_id;
                return new Promise(function(resolve, reject) {
                    globalThis.__obscura_fetch_resolvers[id] = {resolve: resolve, reject: reject};
                    globalThis.__obscura_fetch_queue.push({
                        id: id, url: url, method: method, headers: headers, body: body,
                        origin: origin, mode: mode
                    });
                });
            };"#
        )?;
    }
    {
        // op_sleep: pure JS Promise with setTimeout — no Rust involvement needed
        ctx.eval::<(), _>(
            r#"globalThis.Deno.core.ops.op_sleep = function(millis) {
                return new Promise(function(resolve) { setTimeout(resolve, millis); });
            };"#
        )?;
    }
    {
        // op_web_search: async wrapper using same queue pattern as op_fetch_url
        ctx.eval::<(), _>(
            r#"globalThis.__obscura_search_queue = [];
            globalThis.__obscura_search_id = 0;
            globalThis.__obscura_search_resolvers = {};
            globalThis.Deno.core.ops.op_web_search = function(query) {
                var id = ++globalThis.__obscura_search_id;
                return new Promise(function(resolve, reject) {
                    globalThis.__obscura_search_resolvers[id] = {resolve: resolve, reject: reject};
                    globalThis.__obscura_search_queue.push({ id: id, query: query });
                });
            };"#
        )?;
    }

    Ok(())
}
