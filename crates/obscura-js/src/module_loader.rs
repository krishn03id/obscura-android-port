//! ES module loader — rquickjs port.

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Module};

#[derive(Clone)]
pub struct ObscuraModuleLoader {
    pub base_url: String,
    pub proxy_url: Option<String>,
}

impl ObscuraModuleLoader {
    pub fn new(base_url: &str) -> Self {
        Self::with_proxy(base_url, None)
    }

    pub fn with_proxy(base_url: &str, proxy_url: Option<String>) -> Self {
        ObscuraModuleLoader {
            base_url: base_url.to_string(),
            proxy_url,
        }
    }
}

impl Resolver for ObscuraModuleLoader {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _import_attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let b = if base.is_empty() || base.starts_with('<') || base == "." || base == "about:blank" {
            &self.base_url
        } else {
            base
        };
        let base_url = url::Url::parse(b)
            .map_err(|e| rquickjs::Error::new_from_js_message("url", "resolve", e.to_string()))?;
        let resolved = base_url
            .join(name)
            .map_err(|e| rquickjs::Error::new_from_js_message("url", "resolve", e.to_string()))?;
        Ok(resolved.to_string())
    }
}

impl Loader for ObscuraModuleLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        path: &str,
        _import_attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let proxy_url = self.proxy_url.clone();
        let url = path.to_string();

        let client = crate::ops::cached_request_client(proxy_url.as_deref())
            .map_err(|e| rquickjs::Error::new_from_js_message("net", "load", e))?;

        let rt = tokio::runtime::Handle::current();
        let resp = rt.block_on(async {
            client
                .get(&url)
                .header("Accept", "application/javascript, text/javascript, */*")
                .send()
                .await
                .map_err(|e| rquickjs::Error::new_from_js_message("net", "load", e.to_string()))
        })?;

        if !resp.status().is_success() {
            return Err(rquickjs::Error::new_from_js_message(
                "net",
                "load",
                format!("Module {} returned HTTP {}", url, resp.status()),
            ));
        }

        let code = rt.block_on(async {
            resp.text()
                .await
                .map_err(|e| rquickjs::Error::new_from_js_message("net", "load", e.to_string()))
        })?;

        Module::declare(ctx.clone(), path, code)
    }
}
