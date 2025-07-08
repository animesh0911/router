//! CORS (Cross-Origin Resource Sharing) plugin for Apollo Router
//!
//! This plugin adds CORS headers to HTTP responses and handles preflight OPTIONS requests.
//! It operates at the router service level to ensure CORS is applied to all requests.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use http::StatusCode;
use tokio::sync::Mutex;
use tower::BoxError;
use tower::Layer;
use tower::Service;
use tower::ServiceExt;
use tower_http::cors::CorsLayer;

use crate::configuration::cors::Cors;
use crate::plugin::Plugin;
use crate::plugin::PluginInit;
use crate::register_plugin;
use crate::services::router;

/// CORS plugin
#[derive(Debug, Clone)]
pub(crate) struct CorsPlugin {
    config: Cors,
}

#[async_trait::async_trait]
impl Plugin for CorsPlugin {
    type Config = Cors;

    async fn new(init: PluginInit<Self::Config>) -> Result<Self, BoxError> {
        Ok(CorsPlugin {
            config: init.config,
        })
    }

    fn router_service(&self, service: router::BoxService) -> router::BoxService {
        if self.config.has_trusted_origins() {
            // Use custom CORS handling for trusted origins
            let trusted_cors_service = TrustedOriginsCorsService::new(self.config.clone(), service);
            ServiceExt::boxed(trusted_cors_service)
        } else {
            // Use standard tower-http CORS layer
            match self.config.clone().into_layer() {
                Ok(cors_layer) => {
                    let cors_service = CorsService::new(cors_layer, service);
                    ServiceExt::boxed(cors_service)
                }
                Err(err) => {
                    tracing::error!("Failed to create CORS layer: {}", err);
                    service
                }
            }
        }
    }
}

/// Service that applies CORS to router requests by converting types
#[derive(Clone)]
struct CorsService<S> {
    cors_layer: CorsLayer,
    inner: Arc<Mutex<S>>,
}

impl<S> CorsService<S> {
    fn new(cors_layer: CorsLayer, inner: S) -> Self {
        Self {
            cors_layer,
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<S> Service<router::Request> for CorsService<S>
where
    S: Service<router::Request, Response = router::Response, Error = BoxError> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = router::Response;
    type Error = BoxError;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // For tokio::sync::Mutex, we can't poll_ready synchronously,
        // so we just return Ready and handle the actual readiness in call()
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: router::Request) -> Self::Future {
        // Convert router request to HTTP request
        let http_req: http::Request<router::Body> = req.into();

        // Create HTTP service that wraps the router service
        let inner = self.inner.clone();
        let http_service = tower::service_fn(move |http_req: http::Request<router::Body>| {
            let inner = inner.clone();
            async move {
                let router_req: router::Request = http_req.into();
                let mut inner = inner.lock().await;
                let router_res = inner.ready().await?.call(router_req).await?;
                let http_res: http::Response<router::Body> = router_res.into();
                Ok::<_, BoxError>(http_res)
            }
        });

        // Apply CORS layer to the HTTP service
        let cors_service = self.cors_layer.layer(http_service);

        // Call the CORS service and convert back
        let mut cors_service = cors_service;
        Box::pin(async move {
            let http_res = cors_service.ready().await?.call(http_req).await?;
            let router_res: router::Response = http_res.into();
            Ok(router_res)
        })
    }
}

/// Service that handles trusted origins CORS logic
#[derive(Clone)]
struct TrustedOriginsCorsService<S> {
    config: Cors,
    inner: Arc<Mutex<S>>,
}

impl<S> TrustedOriginsCorsService<S> {
    fn new(config: Cors, inner: S) -> Self {
        Self {
            config,
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Check if the origin is in the trusted list
    fn is_trusted_origin(&self, origin: &str) -> bool {
        if let Some(trusted_origins) = &self.config.trusted_origins {
            trusted_origins.iter().any(|trusted| trusted == origin)
        } else {
            false
        }
    }

    /// Handle preflight OPTIONS request
    fn handle_preflight(&self, req: &router::Request) -> Option<router::Response> {
        let headers = req.router_request.headers();

        // Check if this is a preflight request
        if req.router_request.method() == Method::OPTIONS {
            if let Some(origin) = headers.get("origin") {
                if let Ok(origin_str) = origin.to_str() {
                    let is_trusted = self.is_trusted_origin(origin_str);

                    // Build preflight response
                    let mut response_headers = HeaderMap::new();

                    // Set origin header
                    if is_trusted {
                        response_headers.insert("access-control-allow-origin", origin.clone());
                        response_headers.insert(
                            "access-control-allow-credentials",
                            HeaderValue::from_static("true"),
                        );
                    } else {
                        response_headers
                            .insert("access-control-allow-origin", HeaderValue::from_static("*"));
                    }

                    // Set other CORS headers
                    response_headers.insert(
                        "access-control-allow-methods",
                        HeaderValue::from_str(&self.config.methods.join(", "))
                            .unwrap_or_else(|_| HeaderValue::from_static("GET, POST, OPTIONS")),
                    );

                    if !self.config.allow_headers.is_empty() {
                        response_headers.insert(
                            "access-control-allow-headers",
                            HeaderValue::from_str(&self.config.allow_headers.join(", "))
                                .unwrap_or_else(|_| HeaderValue::from_static("*")),
                        );
                    } else if let Some(requested_headers) =
                        headers.get("access-control-request-headers")
                    {
                        response_headers
                            .insert("access-control-allow-headers", requested_headers.clone());
                    }

                    if let Some(max_age) = &self.config.max_age {
                        response_headers.insert(
                            "access-control-max-age",
                            HeaderValue::from_str(&max_age.as_secs().to_string())
                                .unwrap_or_else(|_| HeaderValue::from_static("86400")),
                        );
                    }

                    // Create response
                    let response = http::Response::builder()
                        .status(StatusCode::OK)
                        .body(router::body::empty())
                        .map_err(|_| "Failed to build preflight response")
                        .ok()?;

                    if let Ok(mut resp) = router::Response::http_response_builder()
                        .response(response)
                        .context(req.context.clone())
                        .build()
                    {
                        // Copy headers to the response
                        for (key, value) in response_headers {
                            if let Some(key) = key {
                                resp.response.headers_mut().insert(key, value);
                            }
                        }
                        return Some(resp);
                    }
                }
            }
        }
        None
    }
}

impl<S> Service<router::Request> for TrustedOriginsCorsService<S>
where
    S: Service<router::Request, Response = router::Response, Error = BoxError> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = router::Response;
    type Error = BoxError;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: router::Request) -> Self::Future {
        // Handle preflight request
        if let Some(preflight_response) = self.handle_preflight(&req) {
            return Box::pin(async move { Ok(preflight_response) });
        }

        let inner = self.inner.clone();
        let config = self.config.clone();

        // Extract origin from request before moving it
        let origin = req
            .router_request
            .headers()
            .get("origin")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        Box::pin(async move {
            let mut inner = inner.lock().await;
            let mut response = inner.ready().await?.call(req).await?;

            // Add CORS headers to the response based on the extracted origin
            let response_headers = response.response.headers_mut();

            if let Some(origin_str) = origin {
                let is_trusted = if let Some(trusted_origins) = &config.trusted_origins {
                    trusted_origins.iter().any(|trusted| trusted == &origin_str)
                } else {
                    false
                };

                if is_trusted {
                    response_headers.insert(
                        "access-control-allow-origin",
                        HeaderValue::from_str(&origin_str)
                            .unwrap_or_else(|_| HeaderValue::from_static("*")),
                    );
                    response_headers.insert(
                        "access-control-allow-credentials",
                        HeaderValue::from_static("true"),
                    );
                } else {
                    response_headers
                        .insert("access-control-allow-origin", HeaderValue::from_static("*"));
                }

                // Add expose headers if configured
                if let Some(expose_headers) = &config.expose_headers {
                    if !expose_headers.is_empty() {
                        if let Ok(expose_value) = HeaderValue::from_str(&expose_headers.join(", "))
                        {
                            response_headers.insert("access-control-expose-headers", expose_value);
                        }
                    }
                }
            } else {
                // No origin header, allow all
                response_headers
                    .insert("access-control-allow-origin", HeaderValue::from_static("*"));
            }

            Ok(response)
        })
    }
}

register_plugin!("apollo", "cors", CorsPlugin);

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use http::Method;
    use http::StatusCode;
    use tower::ServiceExt;

    use super::*;
    use crate::plugin::test::MockRouterService;
    use crate::services::router::Request;
    use crate::services::router::Response;

    #[tokio::test]
    async fn test_cors_plugin_default_config() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let plugin = CorsPlugin::new(PluginInit::fake_new(Cors::default(), Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::GET)
            .header("origin", "https://studio.apollographql.com")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        // Check that CORS headers are present
        let headers = response.response.headers();
        assert!(headers.contains_key("access-control-allow-origin"));
        assert_eq!(
            headers.get("access-control-allow-origin").unwrap(),
            "https://studio.apollographql.com"
        );
    }

    #[tokio::test]
    async fn test_cors_plugin_with_credentials() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let config = Cors::builder()
            .allow_credentials(true)
            .origins(vec!["https://example.com".to_string()])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::GET)
            .header("origin", "https://example.com")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert!(headers.contains_key("access-control-allow-credentials"));
        assert_eq!(
            headers.get("access-control-allow-credentials").unwrap(),
            "true"
        );
    }

    #[tokio::test]
    async fn test_cors_plugin_trusted_origins_trusted_domain() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let config = Cors::builder()
            .trusted_origins(vec!["https://trusted.com".to_string()])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::GET)
            .header("origin", "https://trusted.com")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert_eq!(
            headers.get("access-control-allow-origin").unwrap(),
            "https://trusted.com"
        );
        assert_eq!(
            headers.get("access-control-allow-credentials").unwrap(),
            "true"
        );
    }

    #[tokio::test]
    async fn test_cors_plugin_trusted_origins_untrusted_domain() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let config = Cors::builder()
            .trusted_origins(vec!["https://trusted.com".to_string()])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::GET)
            .header("origin", "https://untrusted.com")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert_eq!(headers.get("access-control-allow-origin").unwrap(), "*");
        assert!(!headers.contains_key("access-control-allow-credentials"));
    }

    #[tokio::test]
    async fn test_cors_plugin_trusted_origins_preflight_trusted() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(0); // Preflight should not reach the service

        let config = Cors::builder()
            .trusted_origins(vec!["https://trusted.com".to_string()])
            .methods(vec![
                "GET".to_string(),
                "POST".to_string(),
                "OPTIONS".to_string(),
            ])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::OPTIONS)
            .header("origin", "https://trusted.com")
            .header("access-control-request-method", "POST")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert_eq!(
            headers.get("access-control-allow-origin").unwrap(),
            "https://trusted.com"
        );
        assert_eq!(
            headers.get("access-control-allow-credentials").unwrap(),
            "true"
        );
        assert!(headers.contains_key("access-control-allow-methods"));
    }

    #[tokio::test]
    async fn test_cors_plugin_trusted_origins_preflight_untrusted() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(0); // Preflight should not reach the service

        let config = Cors::builder()
            .trusted_origins(vec!["https://trusted.com".to_string()])
            .methods(vec![
                "GET".to_string(),
                "POST".to_string(),
                "OPTIONS".to_string(),
            ])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::OPTIONS)
            .header("origin", "https://untrusted.com")
            .header("access-control-request-method", "POST")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert_eq!(headers.get("access-control-allow-origin").unwrap(), "*");
        assert!(!headers.contains_key("access-control-allow-credentials"));
        assert!(headers.contains_key("access-control-allow-methods"));
    }

    #[tokio::test]
    async fn test_cors_plugin_preflight_request() {
        let mut mock_service = MockRouterService::new();
        // For preflight requests, the service might not be called as CORS layer handles it
        mock_service.expect_call().times(0..=1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let config = Cors::builder()
            .allow_headers(vec![
                "content-type".to_string(),
                "authorization".to_string(),
            ])
            .methods(vec![
                "GET".to_string(),
                "POST".to_string(),
                "OPTIONS".to_string(),
            ])
            .max_age(Duration::from_secs(3600))
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::OPTIONS)
            .header("origin", "https://studio.apollographql.com")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "content-type")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert!(headers.contains_key("access-control-allow-origin"));
        assert!(headers.contains_key("access-control-allow-methods"));
        assert!(headers.contains_key("access-control-max-age"));
    }

    #[tokio::test]
    async fn test_cors_plugin_with_match_origins() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let config = Cors::builder()
            .origins(vec!["https://example.com".to_string()])
            .match_origins(vec![r"https://.*\.example\.com".to_string()])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::GET)
            .header("origin", "https://sub.example.com")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert!(headers.contains_key("access-control-allow-origin"));
        assert_eq!(
            headers.get("access-control-allow-origin").unwrap(),
            "https://sub.example.com"
        );
    }

    #[tokio::test]
    async fn test_cors_plugin_expose_headers() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let config = Cors::builder()
            .expose_headers(vec![
                "x-custom-header".to_string(),
                "x-another-header".to_string(),
            ])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::GET)
            .header("origin", "https://studio.apollographql.com")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert!(headers.contains_key("access-control-expose-headers"));
        let expose_headers = headers.get("access-control-expose-headers").unwrap();
        let expose_headers_str = expose_headers.to_str().unwrap();
        assert!(expose_headers_str.contains("x-custom-header"));
        assert!(expose_headers_str.contains("x-another-header"));
    }

    #[tokio::test]
    async fn test_cors_plugin_trusted_origins_expose_headers() {
        let mut mock_service = MockRouterService::new();
        mock_service.expect_call().times(1).returning(|_| {
            Ok(Response::fake_builder()
                .status_code(StatusCode::OK)
                .build()
                .unwrap())
        });

        let config = Cors::builder()
            .trusted_origins(vec!["https://trusted.com".to_string()])
            .expose_headers(vec!["x-custom-header".to_string()])
            .build();

        let plugin = CorsPlugin::new(PluginInit::fake_new(config, Default::default()))
            .await
            .expect("Failed to create CORS plugin");

        let mut service = plugin.router_service(mock_service.boxed());

        let request = Request::fake_builder()
            .method(Method::GET)
            .header("origin", "https://trusted.com")
            .build()
            .unwrap();

        let response = service.ready().await.unwrap().call(request).await.unwrap();

        let headers = response.response.headers();
        assert!(headers.contains_key("access-control-expose-headers"));
        let expose_headers = headers.get("access-control-expose-headers").unwrap();
        let expose_headers_str = expose_headers.to_str().unwrap();
        assert!(expose_headers_str.contains("x-custom-header"));
    }
}
